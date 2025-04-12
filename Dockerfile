FROM node:22 AS builder

WORKDIR /app

# RUN npm set registry https://registry.npmjs.org
COPY package.json /app/package.json
COPY package-lock.json /app/package-lock.json
COPY .npmrc /app/.npmrc
RUN --mount=type=secret,id=ghapikey,required \
    export GH_API_KEY="$(cat /run/secrets/ghapikey)"; \ 
    echo "//npm.pkg.github.com/:_authToken=${GH_API_KEY}" >> ~/.npmrc
RUN npm install

COPY .eslintrc.json /app/.eslintrc.json
COPY .eslintignore /app/.eslintignore
COPY .prettierrc /app/.prettierrc
COPY .prettierignore /app/.prettierignore
COPY tsconfig.json /app/tsconfig.json
COPY config.json /app/config.json
COPY lib /app/lib
COPY __tests__ /app/__tests__
COPY config.json /app/config.json
RUN npm run build

COPY static /app/static

FROM debian:12-slim AS imagemagick

WORKDIR /app
RUN apt update && apt install -y build-essential curl libtool automake autoconf pkg-config libwebp-dev libgd-dev liblcms2-dev libjpeg-dev libpng-dev libtiff-dev libxpm-dev libfreetype6-dev libgif-dev librsvg2-dev libxml2-dev libopenexr-dev
RUN curl -L https://imagemagick.org/archive/ImageMagick.tar.gz | tar xz
RUN cd ImageMagick-* && ./configure --prefix=/app/imbuild --enable-static --disable-shared \
        --with-heic=yes --with-jpeg=yes  --with-png=yes --with-openexr=yes --with-rsvg=yes \
        && make && make install

FROM node:22

WORKDIR /app

ENV MAGICK_CONFIGURE_PATH=/etc/ImageMagick-7

COPY --from=imagemagick /app/imbuild /usr
COPY --from=imagemagick /app/imbuild/etc /etc
COPY --from=builder /app/node_modules /app/node_modules
COPY --from=builder /app/es5 /app/es5
COPY --from=builder /app/static /app/static
COPY --from=builder /app/config.json /app/config.json

USER node

CMD ["node", "es5/server.js"]