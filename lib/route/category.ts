import {
  config,
  LocalNextFunction,
  LocalRequest,
  LocalResponse,
  logging,
} from '@hikari-systems/hs.utils';
import express from 'express';

const log = logging('routes:category');

const router = express.Router();

router.get(
  '/api/category/list',
  (req: LocalRequest, res: LocalResponse, next: LocalNextFunction) => {
    try {
      // Helper function to get size details for a size key
      const getSizeDetails = (sizeKey: string) => {
        const mimeType = config.get(`resize:${sizeKey}:mimeType`);
        if (!mimeType) {
          return null;
        }

        const width = parseInt(`${config.get(`resize:${sizeKey}:width`)}`, 10);
        const height = parseInt(
          `${config.get(`resize:${sizeKey}:height`)}`,
          10,
        );

        return {
          name: sizeKey,
          width: Number.isNaN(width) ? undefined : width,
          height: Number.isNaN(height) ? undefined : height,
          mimeType,
        };
      };

      // Helper function to get sizes for a scaling set (comma-separated size keys)
      const getSizesForScalingSet = (scalingSetStr: string) => {
        const sizeKeys = scalingSetStr
          .split(',')
          .map((k: string) => k.trim())
          .filter((k: string) => k !== '');

        const sizes: Array<{
          name: string;
          width?: number;
          height?: number;
          mimeType: string;
        }> = [];

        sizeKeys.forEach((sizeKey) => {
          const sizeDetails = getSizeDetails(sizeKey);
          if (sizeDetails !== null) {
            sizes.push(sizeDetails);
          }
        });

        return sizes;
      };

      const categories: Array<{
        name: string;
        sizes: Array<{
          name: string;
          width?: number;
          height?: number;
          mimeType: string;
        }>;
      }> = [];

      // Always include default category using resize:sizeKeys
      const defaultSizeKeysStr = config.get('resize:sizeKeys') || '';
      if (defaultSizeKeysStr) {
        const defaultSizes = getSizesForScalingSet(defaultSizeKeysStr);
        if (defaultSizes.length > 0) {
          categories.push({
            name: 'default',
            sizes: defaultSizes,
          });
        }
      }

      // Discover all categories by reading all config keys and filtering for resize:scalingSets:*
      const allConfigKeys = config.getAllKeys();
      const scalingSetPrefix = 'resize:scalingSets:';

      // Filter keys that match the pattern resize:scalingSets:*
      const categoryKeys = allConfigKeys.filter((key) =>
        key.startsWith(scalingSetPrefix),
      );

      // Extract category names and process them
      categoryKeys.forEach((key) => {
        const categoryName = key.substring(scalingSetPrefix.length);
        if (categoryName) {
          const scalingSetStr = config.get(key);
          if (scalingSetStr && typeof scalingSetStr === 'string') {
            const sizes = getSizesForScalingSet(scalingSetStr);
            if (sizes.length > 0) {
              categories.push({
                name: categoryName,
                sizes,
              });
            }
          }
        }
      });

      return res.status(200).json(categories);
    } catch (err) {
      log.error('Error getting category list', err);
      return next(err);
    }
  },
);
export default router;
