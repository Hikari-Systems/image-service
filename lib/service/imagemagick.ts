import { config, logging } from '@hikari-systems/hs.utils';
import { execFile } from 'child_process';
import { tmpName } from 'tmp-promise';

const log = logging('service:imagemagick');

const imageMagick = async (
  sourcePath: string,
  destPath: string,
  w: number,
  h: number,
  extraResizeOpts: string,
): Promise<string> => {
  const command = config.get('imagemagick:bin');
  const resizeArgs: string[] = [
    '-limit',
    'memory',
    `${config.get('resize:memoryLimit') || 32}MiB`,
    '-limit',
    'map',
    `${config.get('resize:mapLimit') || 32}MiB`,
  ];
  if (extraResizeOpts !== '') {
    extraResizeOpts.split(' ').forEach((x) => resizeArgs.push(x));
  }
  if (w > 0 || h > 0) {
    resizeArgs.push('-resize');
    resizeArgs.push(`${w}x${h}`);
  }
  resizeArgs.push(sourcePath);
  resizeArgs.push(destPath);
  log.debug(`Converting ${sourcePath} to ${destPath}`);
  // log.debug(`Command: ${command} ${resizeArgs.join(' ')}`);
  return new Promise((resolve, reject) => {
    execFile(command, resizeArgs, (err: any, stdout: any, stderr: any) => {
      log.debug(`Process output: stdout=${stdout} stderr=${stderr}`);
      if (err) {
        return reject(err);
      }
      log.debug(`Wrote image to file: ${destPath}`);
      return resolve(destPath);
    });
  });
};

export const resizeImage = async (
  source: string,
  w: number,
  h: number,
  extraResizeOpts: string,
  ext: string,
): Promise<string> => {
  log.debug(
    `Resize requested: ${extraResizeOpts} size=${w}x${h}px sourcePath=${source} ext=${ext}`,
  );
  const destPath = await tmpName({ postfix: ext });
  return imageMagick(source, destPath, w, h, extraResizeOpts);
};
