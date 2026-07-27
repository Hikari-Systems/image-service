# ─── Stage 1: ImageMagick (static build) ─────────────────────────────────────
FROM debian:bookworm-slim AS imagemagick

ENV DEBIAN_FRONTEND=noninteractive
WORKDIR /build

RUN apt-get update && apt-get install -y \
    build-essential curl libtool automake autoconf pkg-config \
    libwebp-dev libgd-dev liblcms2-dev libjpeg-dev libpng-dev \
    libtiff-dev libxpm-dev libfreetype6-dev libgif-dev \
    librsvg2-dev libxml2-dev libopenexr-dev \
    && rm -rf /var/lib/apt/lists/*

# imagemagick.org/archive/ moved to GitHub Pages and now 404s, so source comes
# from the GitHub release tags instead. Pinned for reproducible builds — bump
# deliberately rather than tracking latest. `-f` makes curl fail on an HTTP
# error rather than piping an HTML error page into tar.
ARG IMAGEMAGICK_VERSION=7.1.2-28
RUN curl -fL https://github.com/ImageMagick/ImageMagick/archive/refs/tags/${IMAGEMAGICK_VERSION}.tar.gz | tar xz
RUN cd ImageMagick-* && \
    ./configure \
        --prefix=/opt/imagemagick \
        --enable-static \
        --disable-shared \
        --with-heic=yes \
        --with-jpeg=yes \
        --with-png=yes \
        --with-openexr=yes \
        --with-rsvg=yes \
    && make -j$(nproc) && make install

# ─── Stage 2: Rust builder ────────────────────────────────────────────────────
FROM rust:1-bookworm AS builder

WORKDIR /build

# Cache dependency compilation — only reruns when Cargo.toml or Cargo.lock change.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo 'fn main() {}' > src/main.rs
RUN cargo build --release --locked 2>&1 | tail -5
RUN rm -rf src

# Build the application
COPY src ./src
COPY migrations ./migrations
COPY static ./static
COPY config.json ./

# Touch main.rs so cargo re-links against the real source
RUN touch src/main.rs
RUN cargo build --release --locked

# ─── Stage 3: Runtime (debian:bookworm-slim, glibc, no musl) ─────────────────
FROM debian:bookworm-slim AS runtime

# Runtime image libraries required by the ImageMagick static build
RUN apt-get update && apt-get install -y \
    libwebp7 \
    libwebpmux3 \
    libwebpdemux2 \
    libgd3 \
    liblcms2-2 \
    libjpeg62-turbo \
    libpng16-16 \
    libtiff6 \
    libxpm4 \
    libfreetype6 \
    libgif7 \
    librsvg2-2 \
    libxml2 \
    libopenexr-3-1-30 \
    libgomp1 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=imagemagick /opt/imagemagick/bin/magick /usr/local/bin/magick
COPY --from=imagemagick /opt/imagemagick/lib/ImageMagick-* /usr/local/lib/
COPY --from=imagemagick /opt/imagemagick/etc /etc/ImageMagick-7

COPY --from=builder /build/target/release/image-service /app/image-service
COPY --from=builder /build/config.json /app/config.json
COPY --from=builder /build/migrations /app/migrations

ENV MAGICK_CONFIGURE_PATH=/etc/ImageMagick-7

USER nobody

EXPOSE 3000
HEALTHCHECK --interval=10s --timeout=5s --start-period=15s --retries=3 \
    CMD ["/app/image-service", "healthcheck"]

CMD ["/app/image-service"]
