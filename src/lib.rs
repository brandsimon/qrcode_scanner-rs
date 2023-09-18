mod image_decode;

use image::DynamicImage;
use log;
use std::io;
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
	converter: Box<dyn Fn(&[u8], u32, u32) -> io::Result<
		DynamicImage>>,
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
		log::debug!("Choosen camera format: {:?}", format);
		format = dev.set_format(&format)?;
		log::debug!("Camera format set: {:?}", format);
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
		let conv = if format.fourcc == FourCC::new(b"YUYV") {
			image_decode::yuv422_to_image
		} else {
			return Err(io::Error::new(
				io::ErrorKind::InvalidInput,
				"No Camera format supported"));
		};
		return Ok(QRScanStream {
			stream: stream,
			format: format,
			decoder: decoder,
			converter: Box::new(conv),
		});
	}

	pub fn decode_next(self: &mut Self) -> io::Result<Vec<String>> {
		let (buf, _meta) = self.stream.next()?;
		let buf_vec = buf.to_vec();
		let img = (self.converter)(
			&buf_vec,
			self.format.width,
			self.format.height)?;
		let results = self.decoder.decode(&img);
		return Ok(decoded_results_to_vec(results));
	}
}
