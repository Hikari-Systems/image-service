import { config, logging } from '@hikari-systems/hs.utils';
import { v4 } from 'uuid';
import { unlink } from 'fs/promises';
import { tmpName } from 'tmp-promise';
import { imageUpsert } from '../model/image';
import { resizeImage } from '../service/imagemagick';
// import { downloadFromUrl, downloadImage } from '../service/downloader';
import { sanitiseSVG } from '../service/svgSanitiser';
import { ImageType, ScaledImageType } from '../types/types';
import { s3Save } from '../service/s3';
import { getCfSignedUrl } from '../service/cloudfront';
import { downloadImage } from '../service/downloader';

interface ImageTypePlusUrl extends ImageType {
  originalFileUrl: string;
}

const log = logging('helpers:transcodingHelper');

export const getImageDescriptorWithDownloadUrl = async (
  img: ImageType | null,
): Promise<ImageTypePlusUrl | null> => {
  if (!img) {
    log.warn(`WARNING: Image service descriptor was null - aborting`);
    return Promise.resolve(null);
  }
  if (img.downloadedS3Path) {
    log.debug(
      `Using resizedOriginal file s3 path for image: ${JSON.stringify(img)}`,
    );
    const signedUrl = await getCfSignedUrl(img.downloadedS3Path);
    return {
      ...img,
      originalFileUrl: signedUrl,
    };
  }
  if (img.sourceUrl) {
    log.debug(`Using source url for image: ${JSON.stringify(img)}`);
    return {
      ...img,
      originalFileUrl: img.sourceUrl,
    };
  }
  log.warn(`WARNING: Lookup for image ${JSON.stringify(img)} returned null`);
  return null;
};

const buildScaler =
  (sizeKey: string) =>
  async (inFilePath: string, img: ImageType): Promise<ScaledImageType> => {
    const extension = config.get(`resize:${sizeKey}:extension`);
    const scaledImagePath: string = await resizeImage(
      inFilePath,
      parseInt(`${config.get(`resize:${sizeKey}:width`)}`, 10),
      parseInt(`${config.get(`resize:${sizeKey}:height`)}`, 10),
      extension,
    );
    const toSaveAs = `${img.category}-${img.id}-${sizeKey}${extension}`;
    await s3Save(
      scaledImagePath,
      toSaveAs,
      config.get(`resize:${sizeKey}:mimeType`),
    ).finally(() => unlink(scaledImagePath));
    return {
      size: sizeKey,
      s3Path: toSaveAs,
    };
  };

const transcodeImage = async (
  localSourcePath: string,
  toOverwrite: ImageType,
  localFileContentType: string,
): Promise<ImageType> => {
  log.debug(
    `transcodeImage: image=${localSourcePath} toOverwrite=${JSON.stringify(
      toOverwrite,
    )}`,
  );

  // get base vars
  const extension = config.get('resize:original:extension');
  const mimeType = config.get('resize:original:mimeType');
  const category = (toOverwrite?.category || '').toLowerCase();
  const cat = !category || category === '' ? 'image' : category;

  // build image to save: keep only the bits we aren't going to overwrite
  const id = toOverwrite.id || v4();
  const origDestS3Path = `${cat}-${id}-original${extension}`;
  const updatedImage: ImageType = {
    id,
    category: cat,
    avoidResizeUntil: toOverwrite.avoidResizeUntil,
    sourceUrl: toOverwrite.sourceUrl,
    createdAt: toOverwrite.createdAt,
    downloadedS3Path: toOverwrite.downloadedS3Path,
    originalS3Path: origDestS3Path,
  };

  const sizes: string[] = (
    config.get(
      !category || category === ''
        ? 'resize:sizeKeys'
        : `resize:scalingSets:${category}`,
    ) ||
    config.get('resize:sizeKeys') ||
    ''
  ).split(',');
  const scalers = sizes.map((x: string) => buildScaler(x));

  if (localFileContentType === 'image/svg+xml') {
    const sanitisedImagePath = await sanitiseSVG(localSourcePath);
    await s3Save(sanitisedImagePath, origDestS3Path, 'image/svg+xml').finally(
      () => unlink(sanitisedImagePath),
    );
  } else {
    const origResized = await resizeImage(localSourcePath, -1, -1, extension);
    await s3Save(origResized, origDestS3Path, mimeType).finally(() =>
      unlink(origResized),
    );
  }

  // write orig to fileService, then write scaled to fileService
  const scaled: ScaledImageType[] = await Promise.all(
    scalers.map(async (scaler) => scaler(localSourcePath, toOverwrite)),
  );
  return imageUpsert({
    ...updatedImage,
    resizedFiles: scaled.map((x) => ({ size: x.size, s3Path: x.s3Path })),
  });
};

const doProcessImage = async (
  localSourcePath: string,
  sourceExtension: string,
  sourceMimeType: string,
  category: string,
  url: string | null,
  doTranscode: boolean,
  toOverwrite: ImageType | undefined,
): Promise<ImageType> => {
  log.debug(
    `doProcessImage: image=${localSourcePath} cat=${category} url=${url} doTranscode=${doTranscode}`,
  );
  const baseToInsert = toOverwrite || { id: v4() };

  // because we know that we're downloading fresh, clear the old resizes
  // delete baseToInsert.downloadedS3Path;
  // delete baseToInsert.resizedFiles;

  // write orig to fileService, then write scaled to fileService
  const s3Path = `${category}-${baseToInsert.id}${sourceExtension}`;
  await s3Save(localSourcePath, s3Path, sourceMimeType);
  const uploaded = await imageUpsert({
    ...baseToInsert,
    downloadedS3Path: s3Path,
    category,
    sourceUrl: url || undefined,
  });
  if (doTranscode) {
    return transcodeImage(localSourcePath, uploaded, sourceMimeType);
  }
  return uploaded;
};

export const processImage = (
  localSourcePath: string,
  sourceExtension: string,
  sourceMimeType: string,
  category: string,
  url: string | null,
  forceImmediateResize: boolean,
  toOverwrite: ImageType | undefined,
): Promise<ImageType> =>
  doProcessImage(
    localSourcePath,
    sourceExtension,
    sourceMimeType,
    category,
    url,
    forceImmediateResize ||
      (config.get('resize:processing') || 'inline').trim() !== 'deferred',
    toOverwrite,
  );

export const getExtensionFromPath = (path: string): string =>
  `.${path.split('.').pop()}`;

// export const downloadAndAdd = (
//   imgUrl: string,
//   category: string,
//   preferredExtension: string,
//   forceImmediateResize: boolean,
//   toOverwrite: ImageType | undefined,
// ): Promise<ImageType> => {
//   log.debug(`Image url supplied, downloading: ${imgUrl}`);
//   const sourceExtension: string =
//     preferredExtension ||
//     getExtensionFromPath(urlParser.parse(imgUrl).pathname);
//   // const saveExtension: string = config.get('resize:original:extension');

//   return new Promise<ImageType>((resolve, reject) =>
//     tmp.tmpName((errOutImage, imageDestPath) => {
//       if (errOutImage) return reject(errOutImage);

//       return downloadImage(imgUrl, imageDestPath, sourceExtension)
//         .then((image) =>
//           processImage(
//             image.localPath,
//             sourceExtension,
//             image.mimeType,
//             category,
//             imgUrl,
//             forceImmediateResize,
//             toOverwrite,
//           ),
//         )
//         .then(resolve)
//         .catch(reject)
//         .finally(() => fs.unlink(imageDestPath, () => null));
//     }),
//   );
// };

export const fullTranscode = async (
  imageItem: ImageType,
): Promise<ImageType> => {
  // ensure we have a downloaded
  if (!imageItem.downloadedS3Path) {
    if (!imageItem.sourceUrl) {
      throw new Error('No downloaded s3 path and no source url');
    }
    const extension = getExtensionFromPath(
      new URL(imageItem.sourceUrl).pathname,
    );
    const cat =
      (imageItem?.category || '') === '' ? 'image' : imageItem.category;
    const id = imageItem.id || v4();

    const destPath = await tmpName();
    const { localPath, mimeType } = await downloadImage(
      imageItem.sourceUrl || '',
      destPath,
      extension,
    );
    const s3DlPath = `${cat}-${id}${extension}`;
    await s3Save(localPath, s3DlPath, mimeType);
    return transcodeImage(
      localPath,
      { ...imageItem, downloadedS3Path: s3DlPath },
      mimeType,
    ).finally(() => unlink(localPath));
  }

  const extension = getExtensionFromPath(imageItem.downloadedS3Path);
  const dlUrl = await getCfSignedUrl(imageItem.downloadedS3Path);

  const destPath = await tmpName();
  const { localPath, mimeType } = await downloadImage(
    dlUrl,
    destPath,
    extension,
  );
  return transcodeImage(localPath, imageItem, mimeType).finally(() =>
    unlink(localPath),
  );
};
