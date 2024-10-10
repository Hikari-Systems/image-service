import { config, logging } from '@hikari-systems/hs.utils';
import { getSignedUrl } from '@aws-sdk/cloudfront-signer';
import fs from 'fs';
import dayjs from 'dayjs';
import utc from 'dayjs/plugin/utc';

dayjs.extend(utc);

const log = logging('service:cloudfront');

const bucket = config.get('s3:bucketName');
const cfUrl = config.get('cloudfront:url') || '';
const expirySeconds = parseInt(
  config.get('cloudfront:expirySeconds') || '100',
  10,
);

function signerMaker() {
  const keyPairId = config.get(`cloudfront:keypairId`) || '';
  if (keyPairId !== '') {
    const keyText = (config.get(`cloudfront:privateKey`) || '').trim();
    const keyFile = (config.get(`cloudfront:privateKeyFile`) || '').trim();
    if (keyText !== '') {
      log.debug(`Found cloudfront key as config file text`);
      return (s: any) => getSignedUrl({ ...s, keyPairId, privateKey: keyText });
    }
    if (keyFile !== '') {
      log.debug(`Found cloudfront key file: ${keyFile}`);
      const kfText = fs.readFileSync(keyFile, { encoding: 'utf8' });
      return (s: any) => getSignedUrl({ ...s, keyPairId, privateKey: kfText });
    }
  }
  return undefined;
}
const signer = signerMaker();

export const getCfSignedUrl = async (s3Path: string): Promise<string> => {
  const fullCfUrl = `${cfUrl}/${s3Path}`;
  if (!signer) {
    return fullCfUrl;
  }
  const expiresAt = dayjs().utc().add(expirySeconds, 'second');
  const dateLessThan = expiresAt.toDate().toString();
  log.debug(
    `Signing S3 path: ${bucket}${s3Path} as cloudfront url: ${fullCfUrl} expires ${dateLessThan}`,
  );
  return signer({ url: fullCfUrl, dateLessThan });
};
