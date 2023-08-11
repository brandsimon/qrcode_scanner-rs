use std::env;
use qrcode_scanner;

fn main() {
	let args: Vec<String> = env::args().collect();
	if args.len() != 2 {
		println!("Usage: {} /path/to/v4l/device", args[0]);
		return;
	}
	let cam_path = args[1].to_string();
	let qr_stream_res = qrcode_scanner::QRScanStream::new(cam_path);
	let mut qr_stream = match qr_stream_res {
		Ok(q) => q,
		Err(e) => {
			println!("Failed to create QRCodeStream: {}", e);
			return;
		},
	};
	loop {
		let results = match qr_stream.decode_next() {
			Ok(r) => r,
			Err(e) => {
				println!("Failed to decode image: {}", e);
				continue;
			},
		};
		for res in results {
			println!("Found: {}", res);
		}
	}
}
