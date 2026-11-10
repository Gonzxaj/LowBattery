# Reproducible build env: Linux host -> Windows x86_64 (GNU ABI, mingw-w64).
# MSVC is not used: Linux->MSVC needs cargo-xwin + the MS SDK license; GNU covers every dep we use.
FROM rust:1.98.1-slim-bookworm

RUN apt-get update \
 && apt-get install -y --no-install-recommends mingw-w64 pkg-config libwayland-dev \
 && rm -rf /var/lib/apt/lists/* \
 && rustup target add x86_64-pc-windows-gnu \
 && rustup component add clippy rustfmt

ENV CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
WORKDIR /src
