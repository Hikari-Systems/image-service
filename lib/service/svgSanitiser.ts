import { JSDOM } from 'jsdom';
import createDOMPurify from 'dompurify';
import * as fs from 'fs';
import { tmpName } from 'tmp-promise';

const { window } = new JSDOM();
const DOMPurify = createDOMPurify(window);

export const sanitiseSVG = async (filePath: string): Promise<string> => {
  const svgData = fs.readFileSync(filePath);
  const clean = DOMPurify.sanitize(svgData.toString(), {
    USE_PROFILES: { mathMl: true, svg: true },
  });
  const destPath = await tmpName({ postfix: '.svg' });
  fs.writeFileSync(destPath, clean);
  return destPath;
};
