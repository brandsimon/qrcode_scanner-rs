mod image_decode;

use std::collections::VecDeque;
use std::io;

use image::DynamicImage;
use log;
use rxing::{
	common::HybridBinarizer,
	multi::{GenericMultipleBarcodeReader, MultipleBarcodeReader},
	BinaryBitmap, BufferedImageLuminanceSource, DecodeHints, Exceptions,
	MultiUseMultiFormatReader, RXingResult,
};
use v4l::io::traits::CaptureStream;
use v4l::video::Capture;
use v4l::FourCC;

type ConverterFunction =
	Box<dyn Fn(&[u8], u32, u32) -> io::Result<DynamicImage>>;

enum State<'a> {
	V4l {
		stream: v4l::prelude::MmapStream<'a>,
		format: v4l::Format,
		converter: ConverterFunction,
		decoder:
			GenericMultipleBarcodeReader<MultiUseMultiFormatReader>,
	},
	TestImages {
		input_data: VecDeque<(FourCC, u32, u32, Vec<u8>)>,
		decoder:
			GenericMultipleBarcodeReader<MultiUseMultiFormatReader>,
	},
	TestResults {
		results: VecDeque<io::Result<Vec<String>>>,
	},
}

pub struct QRScanStream<'a> {
	state: State<'a>,
}

fn decoded_results_to_vec(
	results: Result<Vec<RXingResult>, Exceptions>,
) -> Vec<String> {
	let mut result = Vec::new();
	let res = match results {
		Ok(r) => r,
		Err(_e) => {
			log::debug!("{}", _e);
			return result;
		}
	};
	for r in res {
		result.push(r.getText().to_string());
	}
	return result;
}

#[derive(Clone, Debug)]
pub struct TargetFrameSize {
	pub width: u32,
	pub height: u32,
}

// lower resolution has faster result
// so choose the smallest which is bigger or equal to the target value
fn choose_framesize(
	mut formats: Vec<(FourCC, v4l::FrameSize)>,
	target: TargetFrameSize,
) -> io::Result<(FourCC, u32, u32)> {
	let mut width = 0;
	let mut height = 0;
	let mut diff = u32::MAX;
	let mut fourcc = FourCC::new(b"0000");
	formats.reverse();
	while let Some((cur_fourcc, framesize)) = formats.pop() {
		for discrete in framesize.size.to_discrete() {
			log::trace!("Available format: {}", discrete);
			let diff_h = target.height.abs_diff(discrete.height);
			let diff_w = target.width.abs_diff(discrete.width);
			let this_diff = diff_h * diff_h
				+ diff_w * diff_w + diff_h * diff_w;
			let use_size = this_diff < diff;
			if use_size {
				width = discrete.width;
				height = discrete.height;
				diff = this_diff;
				fourcc = cur_fourcc.clone();
			}
		}
	}
	if diff == u32::MAX {
		return Err(io::Error::new(
			io::ErrorKind::InvalidInput,
			"No camera format supported",
		));
	}
	return Ok((fourcc, width, height));
}

pub fn empty_test_error() -> io::Result<Vec<String>> {
	return Err(io::Error::new(
		io::ErrorKind::NotFound,
		"End of test data reached",
	));
}

fn choose_and_set_format(
	dev: &v4l::Device,
	target: TargetFrameSize,
) -> io::Result<v4l::Format> {
	let fourccs = vec![FourCC::new(b"YUYV"), FourCC::new(b"MJPG")];
	let mut formats = vec![];
	for fourcc in fourccs {
		for framesize in dev.enum_framesizes(fourcc.clone())? {
			formats.push((fourcc, framesize));
		}
	}
	let (fourcc, width, height) = choose_framesize(formats, target)?;
	let mut format = dev.format()?;
	format.fourcc = fourcc;
	format.height = height;
	format.width = width;
	log::debug!("Chosen camera format: {:?}", format);
	format = dev.set_format(&format)?;
	log::debug!("Camera format set: {:?}", format);
	if format.fourcc != fourcc {
		return Err(io::Error::new(
			io::ErrorKind::InvalidInput,
			"Camera does not support YUYV",
		));
	}
	return Ok(format);
}

fn converter_for_fourcc(fourcc: &FourCC) -> io::Result<ConverterFunction> {
	return Ok(Box::new(if *fourcc == FourCC::new(b"YUYV") {
		image_decode::yuv422_to_image
	} else if *fourcc == FourCC::new(b"MJPG") {
		image_decode::guess_image
	} else {
		return Err(io::Error::new(
			io::ErrorKind::InvalidInput,
			"No Camera format supported",
		));
	}));
}

/// Decode QR-/Barcodes from a camera device
impl<'a> QRScanStream<'a> {
	/// Create a `QRScanStream` from a camera device
	///
	/// The `QRScanStream` will open the camera device, record images and
	/// try to decode QR-Codes and Barcodes from them.
	///
	/// ```
	/// use qrcode_scanner::QRScanStream;
	/// # fn no_v4l_device() {
	/// let mut scanner = QRScanStream::new(
	///     "/dev/video0".to_string()).unwrap();
	/// let res = scanner.decode_next().unwrap();
	/// # }
	pub fn new(path: String) -> io::Result<QRScanStream<'a>> {
		return QRScanStream::with_framesize(
			path,
			TargetFrameSize {
				width: 640,
				height: 480,
			},
		);
	}

	/// Create a `QRScanStream` with a target frame size
	///
	/// The camera device will be configured with a frame size as close
	/// as possible to the target frame size. A bigger frame size needs
	/// longer to process individual images.
	///
	/// ```
	/// use qrcode_scanner::{QRScanStream, TargetFrameSize};
	/// # fn no_v4l_device() {
	/// let target = TargetFrameSize { width: 720, height: 540 };
	/// let mut scanner = QRScanStream::with_framesize(
	///     "/dev/video0".to_string(), target).unwrap();
	/// let res = scanner.decode_next().unwrap();
	/// # }
	pub fn with_framesize(
		path: String,
		target: TargetFrameSize,
	) -> io::Result<QRScanStream<'a>> {
		let mut dev = v4l::Device::with_path(path)?;
		// smaller buffer yields a faster reaction to a changed image
		let buffer_count = 3;
		let format = choose_and_set_format(&dev, target)?;
		let mut stream = v4l::prelude::MmapStream::with_buffers(
			&mut dev,
			v4l::buffer::Type::VideoCapture,
			buffer_count,
		)?;
		stream.next()?; // warmup
		let conv = converter_for_fourcc(&format.fourcc)?;
		let multi_format_reader = MultiUseMultiFormatReader::default();
		let decoder =
			GenericMultipleBarcodeReader::new(multi_format_reader);
		return Ok(QRScanStream {
			state: State::V4l {
				stream: stream,
				format: format,
				converter: conv,
				decoder: decoder,
			},
		});
	}

	/// Create a `QRScanStream` from test images
	///
	/// A call to `decode_next` uses the next image and returns
	/// the data encoded in the image. This can be used to test code
	/// which relies on `QRScanStream`.
	/// ```
	/// # use std::path::Path;
	/// # use std::fs::File;
	/// # use std::io::BufReader;
	/// # use std::io::Read;
	/// # use std::io;
	/// use std::collections::VecDeque;
	/// use v4l::FourCC;
	/// use qrcode_scanner::QRScanStream;
	/// # fn read_file(filename: &str) -> Vec<u8> {
	/// #     let mut path = Path::new(
	/// #         "tests/files/lib").to_path_buf();
	/// #     path.push(filename);
	/// #     let f = File::open(path).unwrap();
	/// #     let mut reader = BufReader::new(f);
	/// #     let mut buffer = Vec::new();
	/// #     reader.read_to_end(&mut buffer).unwrap();
	/// #     return buffer;
	/// # }
	///
	/// let data = VecDeque::from([
	///     (FourCC::new(b"MJPG"), 640, 480, read_file("MJPG_1_in")),
	///     (FourCC::new(b"YUYV"), 640, 480, read_file("YUYV_1_in")),
	/// ]);
	///
	/// let mut scanner = QRScanStream::with_test_images(data).unwrap();
	/// assert_eq!(scanner.decode_next().unwrap(), vec![
	///     "Hello Motion-JPG".to_string()]);
	/// assert_eq!(scanner.decode_next().unwrap(), vec![
	///     "Hello YUYV422".to_string()]);
	/// assert_eq!(
	///     scanner.decode_next().unwrap_err().kind(),
	///     io::ErrorKind::NotFound);
	/// ```
	pub fn with_test_images(
		data: VecDeque<(FourCC, u32, u32, Vec<u8>)>,
	) -> io::Result<QRScanStream<'a>> {
		let multi_format_reader = MultiUseMultiFormatReader::default();
		let decoder =
			GenericMultipleBarcodeReader::new(multi_format_reader);
		return Ok(QRScanStream {
			state: State::TestImages {
				input_data: data,
				decoder: decoder,
			},
		});
	}

	/// Create a `QRScanStream` from test results
	///
	/// A call to `decode_next` uses the next entry and returns it.
	/// This can be used to test code which relies on `QRScanStream`.
	/// ```
	/// use std::collections::VecDeque;
	/// use std::io;
	/// use qrcode_scanner::QRScanStream;
	///
	/// let res1 = vec!["test1".to_string(), "test2".to_string()];
	/// let res2 = Err(io::Error::new(io::ErrorKind::InvalidInput, ""));
	/// let res3 = vec![];
	/// let res4 = vec!["test3".to_string(), "test4".to_string()];
	/// let data = VecDeque::from([
	///     Ok(res1.clone()), res2,
	///     Ok(res3.clone()), Ok(res4.clone())]);
	///
	/// let mut scanner = QRScanStream::with_test_results(data).unwrap();
	/// assert_eq!(scanner.decode_next().unwrap(), res1);
	/// assert_eq!(scanner.decode_next().unwrap_err().kind(),
	///            io::ErrorKind::InvalidInput);
	/// assert_eq!(scanner.decode_next().unwrap(), res3);
	/// assert_eq!(scanner.decode_next().unwrap(), res4);
	/// assert_eq!(
	///     scanner.decode_next().unwrap_err().kind(),
	///     io::ErrorKind::NotFound);
	/// ```
	pub fn with_test_results(
		data: VecDeque<io::Result<Vec<String>>>,
	) -> io::Result<QRScanStream<'a>> {
		return Ok(QRScanStream {
			state: State::TestResults { results: data },
		});
	}

	/// Search the next frame for QR- or Barcodes
	///
	/// This function returns the next QR- or Barcodes found in the next
	/// frame. If no one is found, the `Vec` is empty.
	/// On error, an `io::Error` is returned.
	/// If the `QRScanStream` was initialized with test data, the test
	/// data is returned.
	pub fn decode_next(self: &mut Self) -> io::Result<Vec<String>> {
		let (decoder, img) = match &mut self.state {
			State::TestResults { results } => {
				return match results.pop_front() {
					Some(i) => i,
					None => empty_test_error(),
				};
			}
			State::V4l {
				stream,
				format,
				converter,
				decoder,
			} => {
				let (buf, _meta) = stream.next()?;
				let buf_vec = buf.to_vec();
				let img = (converter)(
					&buf_vec,
					format.width,
					format.height,
				)?;
				(decoder, img)
			}
			State::TestImages {
				decoder,
				input_data,
			} => {
				let data = match input_data.pop_front() {
					Some(d) => d,
					None => {
						return empty_test_error();
					}
				};
				let conv = &converter_for_fourcc(&data.0)?;
				let img = (conv)(&data.3, data.1, data.2)?;
				(decoder, img)
			}
		};
		let results = decoder.decode_multiple_with_hints(
			&mut BinaryBitmap::new(HybridBinarizer::new(
				BufferedImageLuminanceSource::new(img),
			)),
			&DecodeHints::default(),
		);
		return Ok(decoded_results_to_vec(results));
	}
}

#[cfg(test)]
mod tests {
	use v4l::framesize::{Discrete, FrameSize, FrameSizeEnum, Stepwise};
	use v4l::v4l_sys::v4l2_frmsizetypes_V4L2_FRMSIZE_TYPE_DISCRETE;
	use v4l::v4l_sys::v4l2_frmsizetypes_V4L2_FRMSIZE_TYPE_STEPWISE;
	use v4l::FourCC;

	fn add_discrete(
		vec: &mut Vec<(FourCC, v4l::FrameSize)>,
		fourcc: FourCC,
		w: u32,
		h: u32,
	) {
		let size = FrameSize {
			index: vec.len() as u32,
			fourcc: fourcc,
			typ: v4l2_frmsizetypes_V4L2_FRMSIZE_TYPE_DISCRETE,
			size: FrameSizeEnum::Discrete(Discrete {
				width: w,
				height: h,
			}),
		};
		vec.push((fourcc, size));
	}

	fn choose_framesize_input(c: usize) -> Vec<(FourCC, v4l::FrameSize)> {
		let four_a = FourCC::new(b"AAAD");
		let four_b = FourCC::new(b"BBBE");
		let four_c = FourCC::new(b"CCCF");
		let mut input = vec![];
		add_discrete(&mut input, four_a, 640, 80);
		add_discrete(&mut input, four_b, 640, 80);
		add_discrete(&mut input, four_b, 480, 200);
		add_discrete(&mut input, four_a, 580, 400);
		add_discrete(&mut input, four_b, 680, 500);
		add_discrete(&mut input, four_a, 720, 490);
		let size = FrameSize {
			index: input.len() as u32,
			fourcc: four_c,
			typ: v4l2_frmsizetypes_V4L2_FRMSIZE_TYPE_STEPWISE,
			size: FrameSizeEnum::Stepwise(Stepwise {
				min_width: 80,
				max_width: 1920,
				step_width: 40,
				min_height: 80,
				max_height: 1080,
				step_height: 40,
			}),
		};
		input.push((four_c, size));
		while input.len() > c {
			input.pop();
		}
		return input;
	}

	#[test]
	fn choose_framesize() {
		let target = super::TargetFrameSize {
			width: 640,
			height: 480,
		};
		let result = super::choose_framesize(vec![], target.clone());
		assert_eq!(
			result.unwrap_err().kind(),
			std::io::ErrorKind::InvalidInput
		);

		let four_a = FourCC::new(b"AAAD");
		let four_b = FourCC::new(b"BBBE");
		let four_c = FourCC::new(b"CCCF");
		assert_eq!(
			super::choose_framesize(
				choose_framesize_input(2),
				target.clone()
			)
			.unwrap(),
			(four_a, 640, 80)
		);
		assert_eq!(
			super::choose_framesize(
				choose_framesize_input(3),
				target.clone()
			)
			.unwrap(),
			(four_b, 480, 200)
		);
		assert_eq!(
			super::choose_framesize(
				choose_framesize_input(4),
				target.clone()
			)
			.unwrap(),
			(four_a, 580, 400)
		);
		assert_eq!(
			super::choose_framesize(
				choose_framesize_input(5),
				target.clone()
			)
			.unwrap(),
			(four_b, 680, 500)
		);
		assert_eq!(
			super::choose_framesize(
				choose_framesize_input(6),
				target.clone()
			)
			.unwrap(),
			(four_b, 680, 500)
		);
		assert_eq!(
			super::choose_framesize(
				choose_framesize_input(7),
				target.clone()
			)
			.unwrap(),
			(four_c, 640, 480)
		);
		assert_eq!(
			super::choose_framesize(
				choose_framesize_input(7),
				super::TargetFrameSize {
					width: 720,
					height: 485,
				}
			)
			.unwrap(),
			(four_a, 720, 490)
		);
	}
}
