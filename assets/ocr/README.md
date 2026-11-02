# Bundled offline OCR models

Source: `@rustors/model-ppocrv6-tiny` version **0.3.2**.
Package: https://registry.npmjs.org/@rustors/model-ppocrv6-tiny/-/model-ppocrv6-tiny-0.3.2.tgz
Upstream: https://github.com/byrizki/rusto-rs
The package declares MIT; its original notice is preserved as `LICENSE.txt`.
The archive SHA-512 was verified against the registry integrity value before extraction.

These public, multilingual PP-OCRv6 Tiny models run through RustO/RTen.
They are embedded in the executable. No OS OCR service, Python, Tesseract,
C++ inference runtime or model download is needed at application startup.
The model files are temporarily materialized for initialization because the
upstream loader takes file paths. Page images stay in process memory.

## SHA-256

- `det.rten`: `ca1f0196d2a2da2b485e2cda6aaf5ba02258ac71127d577c8a77a75e89b67605`
- `rec.rten`: `6c94c6b42c43a39d11cf3dd1267199ba04a11afe9de43632daa6f3983b365a9b`
- `dict.txt`: `c5cbe34ef40c29c4df07ed012bf96569cb69a2d2a01a07027e9f13cb832bd9cd`
