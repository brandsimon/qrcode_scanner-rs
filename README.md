# qrcode_scanner-rs

## Scan QR-codes with video4linux devices

This repository provides a `QRScanStream` struct to scan QR-codes from
video4linux devices which support the YUYV or Motion-JPG format.
It uses `v4l` to get images from the camera and `bardecoder` to extract
the QR-code data.
An example is provided in `src/main.rs`.
