use std::io;
use std::io::Cursor;

use ffimage::color::Rgb;
use ffimage::iter::{BytesExt, ColorConvertExt, PixelsExt};
use ffimage_yuv::{yuv::Yuv, yuv422::Yuv422};
use image::DynamicImage;
use image::ImageReader;

pub fn image_decode_error() -> io::Result<DynamicImage> {
	return Err(io::Error::new(
		io::ErrorKind::InvalidInput,
		"Failed to convert to image",
	));
}

pub fn yuv422_to_image(
	src: &[u8],
	width: u32,
	height: u32,
) -> io::Result<DynamicImage> {
	let mut rgb = vec![0; (width * height * 3) as usize];
	src.iter()
		.copied()
		.pixels::<Yuv422<u8, 0, 2, 1, 3>>()
		.colorconvert::<[Yuv<u8>; 2]>()
		.flatten()
		.colorconvert::<Rgb<u8>>()
		.bytes()
		.write(&mut rgb);
	match <image::RgbImage>::from_vec(width, height, rgb) {
		Some(i) => {
			return Ok(DynamicImage::ImageRgb8(i));
		}
		None => return image_decode_error(),
	};
}

pub fn guess_image(
	src: &[u8],
	_width: u32,
	_height: u32,
) -> io::Result<DynamicImage> {
	let img_reader = ImageReader::new(Cursor::new(src));
	let img_reader_guess = match img_reader.with_guessed_format() {
		Ok(i) => i,
		Err(e) => {
			log::debug!("{}", e);
			return image_decode_error();
		}
	};
	return match img_reader_guess.decode() {
		Ok(i) => Ok(i),
		Err(e) => {
			log::debug!("{}", e);
			image_decode_error()
		}
	};
}

#[cfg(test)]
mod tests {
	use std::fs::File;
	use std::io;
	use std::io::{BufReader, Read};
	use std::path::Path;

	fn read_file(filename: &str) -> io::Result<Vec<u8>> {
		let mut path =
			Path::new("tests/files/image_decode").to_path_buf();
		path.push(filename);
		let f = File::open(path)?;
		let mut reader = BufReader::new(f);
		let mut buffer = Vec::new();
		reader.read_to_end(&mut buffer)?;
		return Ok(buffer);
	}

	#[test]
	fn guess_image_mjpg() {
		let data = read_file("MJPG_1_in").unwrap();
		let result = super::guess_image(&data, 0, 0).unwrap();
		assert_eq!(
			result.into_rgb8().into_raw(),
			read_file("MJPG_1_raw").unwrap()
		);
	}

	#[test]
	fn yuv422_to_image() {
		let data = read_file("YUYV_1_in").unwrap();
		let result = super::yuv422_to_image(&data, 640, 480).unwrap();
		assert_eq!(
			result.into_rgb8().into_raw(),
			read_file("YUYV_1_raw").unwrap()
		);
	}
}
