FROM node:22 AS builder

WORKDIR /app

# RUN npm set registry https://registry.npmjs.org
COPY package.json /app/package.json
COPY package-lock.json /app/package-lock.json
COPY .npmrc /app/.npmrc
RUN npm ci

COPY .eslintrc.json .eslintignore .prettierrc .prettierignore tsconfig.json config.json /app/
COPY lib /app/lib
COPY __tests__ /app/__tests__

RUN npm run build

COPY esbuild.config.js /app/esbuild.config.js
RUN npm run build:esbuild

COPY static /app/static

FROM debian:12-slim AS imagemagick

ENV DEBIAN_FRONTEND=noninteractive
WORKDIR /app
RUN apt update && apt install -y build-essential curl libtool automake autoconf pkg-config libwebp-dev libgd-dev liblcms2-dev libjpeg-dev libpng-dev libtiff-dev libxpm-dev libfreetype6-dev libgif-dev librsvg2-dev libxml2-dev libopenexr-dev
RUN curl -L https://imagemagick.org/archive/ImageMagick.tar.gz | tar xz
RUN cd ImageMagick-* && ./configure --prefix=/app/imbuild --enable-static --disable-shared \
        --with-heic=yes --with-jpeg=yes  --with-png=yes --with-openexr=yes --with-rsvg=yes \
        && make && make install

        # --- Runtime image ---
FROM debian:12-slim

WORKDIR /app
ENV MAGICK_CONFIGURE_PATH=/etc/ImageMagick-7

# Install runtime image libraries
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
    && rm -rf /var/lib/apt/lists/*

# Copy only the node binary and required assets
COPY --from=imagemagick /app/imbuild /usr
COPY --from=imagemagick /app/imbuild/etc /etc
COPY --from=builder /usr/local/bin/node /app/node
COPY --from=builder /app/dist/* /app/dist/
COPY --from=builder /app/static /app/static
COPY --from=builder /app/config.json /app/config.json
RUN echo 'module.exports = {};' > /app/dist/xhr-sync-worker.js

USER nobody

CMD ["/app/node", "/app/dist/server.bundle.js"]