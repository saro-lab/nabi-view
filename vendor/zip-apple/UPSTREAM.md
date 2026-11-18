# zip 9.5.1 for Nabi View on Apple platforms

Source: https://crates.io/api/v1/crates/zip/9.5.1/download
SHA-256 of the published crate: `e3476fd497ac8af5b1ec7b51acab7bef380f7e8dd8e0487595bafd839417a302`.
Upstream repository: https://github.com/zip-rs/zip2
License: MIT (see LICENSE).

Nabi View uses this copy only for macOS and iOS. The package is named
`zip-apple`; its library API still uses the upstream `zip` name. The local
`zipcrypto` feature gates the upstream legacy ZIP cipher module and its
reader/writer branches. The application enables neither `zipcrypto` nor
`aes-crypto`. The plain ZIP codecs remain the same. All other platforms
use the original registry package.
