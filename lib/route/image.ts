import {
  config,
  LocalNextFunction,
  LocalRequest,
  LocalResponse,
  logging,
} from '@hikari-systems/hs.utils';
import express from 'express';
import multer from 'multer';
import autoReap from '../helpers/multer-autoreap';
import { imageGet } from '../model/image';
import {
  getImageDescriptorWithDownloadUrl,
  // downloadAndAdd,
  fullTranscode,
  processImage,
  getExtensionFromPath,
} from '../helpers/transcode';
import { ScaledImageType } from '../types/types';
import { getCfSignedUrl } from '../service/cloudfront';

const log = logging('routes:image');

const upload = multer({ dest: config.get('uploadDir') });
const router = express.Router();

router.get(
  '/api/image/:id',
  (req: LocalRequest, res: LocalResponse, next: LocalNextFunction) => {
    const { id } = req.params;
    log.debug(`Get by id: ${id}`);
    return imageGet(id)
      .then(getImageDescriptorWithDownloadUrl)
      .then((f) =>
        f ? res.status(200).json(f) : res.status(404).send('Not found'),
      )
      .catch((err) => {
        log.error(`Error getting image id=${id}`, err);
        return next(err);
      });
  },
);

// router.get('/api/image/bycategory/:category', (req, res, next) => {
//   const { category } = req.params;
//   log.debug(`Get by category: ${category}`);
//   return imageModel
//     .getByCategory(category)
//     .then((desc) =>
//       desc ? res.status(200).json(desc) : res.status(404).send('Not found'),
//     )
//     .catch((err) => {
//       log.error(`Error getting image by category=${category}`, err);
//       return next(err);
//     });
// });

router.get(
  '/api/image/r/:id/:size',
  async (req: LocalRequest, res: LocalResponse, next: LocalNextFunction) => {
    const { id, size } = req.params;
    log.debug(`Get image ${id} resized as ${size}`);
    try {
      const image = await imageGet(id);
      if (!image) {
        log.debug(`Image not found: ${id}`);
        return res.status(404).send(`Image not found: ${id}`);
      }

      const sc = (image.resizedFiles || []).filter(
        (x: ScaledImageType) => x.size === size,
      );
      if (sc.length > 0) {
        log.debug(
          `Resized version found for size ${size} in ${JSON.stringify(image)}`,
        );
        const signed = await getCfSignedUrl(sc[0].s3Path);
        return res.redirect(signed);
      }
      if (image.originalS3Path) {
        log.debug(
          `No resized version found in ${JSON.stringify(
            image,
          )} - serving as original`,
        );
        const signed = await getCfSignedUrl(image.originalS3Path);
        return res.redirect(signed);
      }
      if (image.downloadedS3Path) {
        log.debug(
          `No processed version found in ${JSON.stringify(
            image,
          )} - serving as downloaded`,
        );
        const signed = await getCfSignedUrl(image.downloadedS3Path);
        return res.redirect(signed);
      }
      if (image.sourceUrl && image.sourceUrl !== '') {
        log.debug(
          `No downloaded or processed version found in ${JSON.stringify(
            image,
          )} - serving as source url`,
        );
        return res.redirect(image.sourceUrl);
      }
      throw new Error(`No usable image data for image ${id}`);
    } catch (err) {
      log.error(`Error getting image id=${id}`, err);
      return next(err);
    }
  },
);

router.post(
  '/api/image/:id/transcode',
  async (req: LocalRequest, res: LocalResponse, next: LocalNextFunction) => {
    const { id } = req.params;

    log.debug(`Downloading to do transcoding: id=${id}`);
    try {
      const img = await imageGet(id);
      if (img?.downloadedS3Path) {
        const t = await fullTranscode(img);
        return res.status(200).json(t);
      }
      return res.status(404);
    } catch (err) {
      log.error(`Error transcoding image ${id}`, err);
      return next(err);
    }
  },
);

// router.post(
//   '/api/image/:category/url',
//   (req: LocalRequest, res: LocalResponse, next: LocalNextFunction) => {
//     const { url, extension, forceImmediateResize } = req.query as {
//       url: string;
//       extension: string;
//       forceImmediateResize: string;
//     };
//     const { category } = req.params;
//     const force = ((req.query.force as string) || 'no').trim() === 'yes';
//     const forceImmediateResizeBool: boolean =
//       ((forceImmediateResize as string) || 'no').trim() === 'yes';
//     return imageListByUrlAndCategory(url, category)
//       .then((found: ImageType[]): Promise<ImageType> => {
//         if (found.length === 0) {
//           log.debug(`URL not found in DB - downloading: ${url}`);
//           return downloadAndAdd(
//             url as string,
//             category,
//             extension as string,
//             forceImmediateResizeBool,
//             undefined,
//           );
//         }
//         if (force) {
//           log.debug(`URL found in DB, but forced downloading: ${url}`);
//           return downloadAndAdd(
//             url as string,
//             category,
//             extension as string,
//             forceImmediateResizeBool,
//             found[0],
//           );
//         }
//         log.debug(
//           `URL found in DB, and no forced update so returning: ${url} ${JSON.stringify(
//             found[0],
//           )}`,
//         );
//         return Promise.resolve(found[0]);
//       })
//       .then((added: ImageType) => res.status(201).json(added))
//       .catch((err: Error) => {
//         log.error(`Error downloading image from url: ${url}`, err);
//         return next(err);
//       });
//   },
// );

router.post(
  '/api/image/:category',
  upload.single('image'),
  (req: LocalRequest, res: LocalResponse, next: LocalNextFunction) => {
    if (!req.file) {
      log.error('ERROR: no image file supplied');
      return res.status(400).send('No image file supplied');
    }
    const { forceImmediateResize } = req.query as {
      forceImmediateResize: string;
    };
    const forceImmediateResizeBool: boolean =
      forceImmediateResize?.toLowerCase().trim() === 'true';
    log.debug(
      `Image uploaded: path=${req.file.path} forceImmediateResize=${forceImmediateResizeBool}`,
    );
    return processImage(
      req.file.path,
      getExtensionFromPath(req.file.originalname),
      req.file.mimetype,
      req.params.category,
      null,
      forceImmediateResizeBool,
      undefined,
    )
      .then((added) => {
        // @ts-expect-error:next-line
        req.added = added;
        return next();
      })
      .catch((err) => {
        log.error('Error uploading image', err);
        // @ts-expect-error:next-line
        req.myErr = err;
        return next();
      });
  },
  autoReap({}),
  (req: any, res, next) => {
    if (req.myErr) {
      return next(req.myErr);
    }
    return res.status(201).json(req.added);
  },
);

export default router;
