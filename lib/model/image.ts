import { config, logging } from '@hikari-systems/hs.utils';
import { v4 } from 'uuid';
import { readFile } from 'fs';
import { writeFile } from 'fs/promises';
import { ImageType } from '../types/types';

const log = logging('model:image');
const imagePath = config.get('imageMetadata:parentPath');

export const imageGet = async (id: string): Promise<ImageType | null> =>
  new Promise((resolve, reject) => {
    readFile(
      `${imagePath}/${id}.json`,
      {
        encoding: 'utf-8',
      },
      (err, json) => {
        if (err && err.code !== 'ENOENT') {
          log.error(`Error loading image details for image ID=${id}`, err);
          reject(err);
        } else if (err) {
          resolve(null);
        } else {
          resolve(JSON.parse(json) as ImageType);
        }
      },
    );
  });

export const imageUpsert = async (image: ImageType): Promise<ImageType> => {
  const rewritten = { ...image, id: image?.id || v4() } as ImageType;
  try {
    await writeFile(
      `${imagePath}/${rewritten.id}.json`,
      JSON.stringify(rewritten),
      {
        encoding: 'utf-8',
      },
    );
    return rewritten;
  } catch (err) {
    log.error(
      `Error saving image details for image=${JSON.stringify(rewritten)}`,
      err,
    );
    throw err;
  }
};
// export const imageListByUrlAndCategory = async (
//   url: string,
//   category: string,
// ): Promise<ImageType[]> => [] as ImageType[];
