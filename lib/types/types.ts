export interface ScaledImageType {
  size: string;
  s3Path: string;
}

export interface ImageType {
  id?: string;
  category?: string;
  sourceUrl?: string;
  downloadedS3Path?: string;
  originalS3Path?: string;
  resizedFiles?: ScaledImageType[];
  avoidResizeUntil?: Date;
  createdAt?: Date;
  // updatedAt?: Date;
}
