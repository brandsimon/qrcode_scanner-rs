use std::io;

use ffimage::color::Rgb;
use ffimage::packed::{ImageBuffer, ImageView};
use ffimage::traits::Convert;
use ffimage_yuv::{yuv::Yuv, yuyv::Yuyv};
use image::DynamicImage;
use image::io::Reader as ImageReader;
use std::io::Cursor;


pub fn image_decode_error() -> io::Result<DynamicImage> {
	return Err(io::Error::new(
		io::ErrorKind::InvalidInput,
		"Failed to convert to image"));
}

pub fn yuv422_to_image(src: &[u8], width: u32, height: u32)
-> io::Result<DynamicImage> {
	let img = ImageView::<Yuyv<u8>>::from_buf(src, width, height);
	let yuv422 = match img {
		Some(view) => view,
		None => return image_decode_error(),
	};
	let mut yuv444 = ImageBuffer::<Yuv<u8>>::new(width, height, 0u8);
	yuv422.convert(&mut yuv444);
	let mut rgb = ImageBuffer::<Rgb<u8>>::new(width, height, 0u8);
	yuv444.convert(&mut rgb);
	match <image::RgbImage>::from_vec(width, height, rgb.into_buf()) {
		Some(i) => {
			return Ok(DynamicImage::ImageRgb8(i));
		},
		None => return image_decode_error(),
	};
}

pub fn guess_image(src: &[u8], _width: u32, _height: u32)
-> io::Result<DynamicImage> {
	let img_reader = ImageReader::new(Cursor::new(src));
	let img_reader_guess = match img_reader.with_guessed_format() {
		Ok(i) => i,
		Err(e) => {
			log::debug!("{}", e);
			return image_decode_error();
		},
	};
	return match img_reader_guess.decode() {
		Ok(i) => Ok(i),
		Err(e) => {
			log::debug!("{}", e);
			image_decode_error()
		},
	};
}

#[cfg(test)]
mod tests {
	use std::io;
	use std::io::{BufReader, Read};
	use std::fs::File;
	use std::path::Path;

	fn read_file(filename: &str) -> io::Result<Vec<u8>> {
		let mut path = Path::new(
			"tests/files/image_decode").to_path_buf();
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
		assert_eq!(result.into_rgb8().into_raw(), read_file(
			"MJPG_1_raw").unwrap());
	}

	#[test]
	fn yuv422_to_image() {
		let data = read_file("YUYV_1_in").unwrap();
		let result = super::yuv422_to_image(&data, 640, 480).unwrap();
		assert_eq!(result.into_rgb8().into_raw(), read_file(
			"YUYV_1_raw").unwrap());
	}
}
