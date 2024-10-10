import { logging } from '@hikari-systems/hs.utils';
import { createWriteStream } from 'fs';

const log = logging('service:downloader');

export interface DownloadedFileType {
  localPath: string;
  mimeType: string;
}

export const downloadImage = async (
  imgUrl: string,
  dest: string,
  extension: string,
): Promise<DownloadedFileType> => {
  const fullDest = `${dest}${extension}`;
  log.debug(`Downloading image from ${imgUrl} to ${fullDest}`);
  try {
    const response = await fetch(imgUrl);
    if (response.status !== 200) {
      throw new Error(
        `Error downloading image: ${imgUrl} to ${dest}: status=${response.status}`,
      );
    }
    const blob = await response.blob();
    const readStream = blob.stream();
    const writeStream = createWriteStream(fullDest);
    const writeableStream = new WritableStream<Uint8Array>({
      write(chunk) {
        writeStream.write(chunk);
        return Promise.resolve();
      },
      close() {
        writeStream.close();
        return Promise.resolve();
      },
    });
    await readStream.pipeTo(writeableStream);
    return {
      localPath: fullDest,
      mimeType:
        response.headers.get('content-type') || 'application/octet-stream',
    };
  } catch (err) {
    log.error(`Error downloading image: ${imgUrl} to ${dest}`, err);
    throw err;
  }
};
