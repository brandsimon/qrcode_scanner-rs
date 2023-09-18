use std::io;

use ffimage::color::Rgb;
use ffimage::packed::{ImageBuffer, ImageView};
use ffimage::traits::Convert;
use ffimage_yuv::{yuv::Yuv, yuyv::Yuyv};
use image::DynamicImage;


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
