import fs from 'fs';
import { config } from '@hikari-systems/hs.utils';
import { S3Client, PutObjectCommand } from '@aws-sdk/client-s3';

export const s3Save = (
  from: string,
  to: string,
  mimeType = 'application/octet-stream',
): Promise<void> =>
  new S3Client({
    credentials: {
      accessKeyId: config.get(`s3:accessKeyId`),
      secretAccessKey: config.get(`s3:secretAccessKey`),
    },
    region: config.get(`s3:region`),
  })
    .send(
      new PutObjectCommand({
        Bucket: config.get('s3:bucketName'),
        Key: to,
        Body: fs.readFileSync(from),
        ContentType: mimeType,
      }),
    )
    .then(() => undefined);

// export const s3Get = (path: string): Promise<void> =>
//   new S3Client({
//     credentials: {
//       accessKeyId: config.get(`s3:accessKeyId`),
//       secretAccessKey: config.get(`s3:secretAccessKey`),
//     },
//     region: config.get(`s3:region`),
//   })
//     .send(
//       new GetObjectCommand({
//         Bucket: config.get('s3:bucketName'),
//         Key: path,
//       }),
//     )
//     .then(() => undefined);
