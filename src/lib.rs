use std::io;

use ffimage::color::Rgb;
use ffimage::packed::{ImageBuffer, ImageView};
use ffimage::traits::Convert;
use ffimage_yuv::{yuv::Yuv, yuyv::Yuyv};
use log;
use v4l::FourCC;
use v4l::io::traits::CaptureStream;
use v4l::video::Capture;


type DefaultDecoder = bardecoder::Decoder<
	image::DynamicImage,
	image::ImageBuffer<image::Luma<u8>, Vec<u8>>,
	String>;

pub struct QRScanStream<'a> {
	stream: v4l::prelude::MmapStream<'a>,
	format: v4l::Format,
	decoder: DefaultDecoder,
}

fn decoded_results_to_vec(results: Vec<Result<String, anyhow::Error>>)
-> Vec<String> {
	let mut result = Vec::new();
	for r in results {
		match r {
			Ok(inner) => {
				result.push(inner);
			},
			Err(_e) => {
				log::debug!("{}", _e);
			},
		};
	}
	return result;
}

fn yuv422_to_image(src: &[u8], width: u32, height: u32)
-> io::Result<Vec<u8>> {
	let img = ImageView::<Yuyv<u8>>::from_buf(src, width, height);
	let yuv422 = match img {
		Some(view) => view,
		None => return Err(io::Error::new(
			io::ErrorKind::InvalidInput,
			"Failed to convert to yuv422")),
	};
	let mut yuv444 = ImageBuffer::<Yuv<u8>>::new(width, height, 0u8);
	yuv422.convert(&mut yuv444);
	let mut rgb = ImageBuffer::<Rgb<u8>>::new(width, height, 0u8);
	yuv444.convert(&mut rgb);
	return Ok(rgb.into_buf());
}

// lower resolution has faster result
// so choose the smallest which is bigger or equal to the target value
fn calc_framesize(dev: &v4l::Device, fourcc: &FourCC)
-> io::Result<(u32, u32)> {
	let mut width = 0;
	let mut height = 0;
	let mut size = 0;
	let target = 640 * 480;
	for framesize in dev.enum_framesizes(fourcc.clone())? {
		for discrete in framesize.size.to_discrete() {
			log::trace!("Available format: {}", discrete);
			let cur_size = discrete.width * discrete.height;
			let use_size =
				(size > cur_size && cur_size > target) ||
				(cur_size > size && size < target);
			if use_size {
				width = discrete.width;
				height = discrete.height;
				size = cur_size;
			}
		}
	}
	return Ok((width, height));
}

impl<'a> QRScanStream<'a> {
	pub fn new(path: String) -> io::Result<QRScanStream<'a>> {
		let mut dev = v4l::Device::with_path(path)?;
		let buffer_count = 30;
		let mut format = dev.format()?;
		let fourcc = FourCC::new(b"YUYV");
		format.fourcc = fourcc;
		let (width, height) = calc_framesize(&dev, &fourcc)?;
		format.height = height;
		format.width = width;
		format = dev.set_format(&format)?;
		log::debug!("Camera format: {:?}", format);
		if format.fourcc != fourcc {
			return Err(io::Error::new(
				io::ErrorKind::InvalidInput,
				"Camera does not support YUYV"));
		}
		let mut stream = v4l::prelude::MmapStream::with_buffers(
			&mut dev,
			v4l::buffer::Type::VideoCapture,
			buffer_count)?;
		let decoder = bardecoder::default_decoder();
		stream.next()?; // warmup
		return Ok(QRScanStream {
			stream: stream,
			format: format,
			decoder: decoder,
		});
	}

	pub fn decode_next(self: &mut Self) -> io::Result<Vec<String>> {
		let (buf, _meta) = self.stream.next()?;
		let buf_vec = buf.to_vec();
		let rgb_buf = yuv422_to_image(
			&buf_vec,
			self.format.width,
			self.format.height)?;
		let image_option = <image::RgbImage>::from_vec(
			self.format.width, self.format.height, rgb_buf);
		match image_option {
			Some(img_buf) => {
				let image = image::DynamicImage::ImageRgb8(
					img_buf);
				let results = self.decoder.decode(&image);
				return Ok(decoded_results_to_vec(results));
			},
			None => {
				return Ok(vec![]);
			},
		}
	}
}
