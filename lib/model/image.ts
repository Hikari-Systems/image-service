import { config } from '@hikari-systems/hs.utils';
import knex, { Knex } from 'knex';
import { ImageType } from '../types/types';
import { imageGet as fileGet, imageUpsert as fileUpsert } from './image-file';
import imageDbFactory from './image-db';
import knexConfig from '../knexfile';

interface ImageBackend {
  get: (id: string) => Promise<ImageType | null>;
  upsert: (image: ImageType) => Promise<ImageType>;
}

const getBackend = (() => {
  let backend: ImageBackend | null = null;
  return (): ImageBackend => {
    if (!backend) {
      const storage = (config.get('imageMetadata:storage') || 'file').trim();
      if (storage === 'db') {
        const db: Knex = knex(knexConfig.main);
        const model = imageDbFactory(db);
        backend = { get: model.get, upsert: model.upsert };
      } else {
        backend = { get: fileGet, upsert: fileUpsert };
      }
    }
    return backend;
  };
})();

export const imageGet = (id: string): Promise<ImageType | null> =>
  getBackend().get(id);

export const imageUpsert = (image: ImageType): Promise<ImageType> =>
  getBackend().upsert(image);
