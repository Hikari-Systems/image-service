# Environment for running image-service on the host against the MinIO container.
#
#   docker compose up -d minio minio-init
#   source scripts/local-env.sh
#   cargo run
#
# MinIO stands in for S3, and CloudFront signing is left off (no keypairId), so the
# URLs the API returns point straight at MinIO and are fetchable with curl. That is
# what makes POST /api/image/:id/transcode work locally — it re-downloads the source
# through that URL.

export log__level=debug
export server__port=3000

# Metadata on disk rather than Postgres — no second container needed.
export imageMetadata__storage=file
export imageMetadata__parentPath="${PWD}/.local/metadata"
mkdir -p "${imageMetadata__parentPath}"

export s3__endpointUrl=http://localhost:9000
export s3__bucketName=image-service-local
export s3__accessKeyId=minioadmin
export s3__secretAccessKey=minioadmin
export s3__region=us-east-1

export cloudfront__url=http://localhost:9000/image-service-local

# ImageMagick 7 ships as `magick`; IM6's `convert` takes the same arguments, so use
# whichever this machine has.
if command -v magick >/dev/null 2>&1; then
  export imagemagick__bin="$(command -v magick)"
elif command -v convert >/dev/null 2>&1; then
  export imagemagick__bin="$(command -v convert)"
else
  echo "warning: no ImageMagick binary found — transcoding will fail" >&2
fi

echo "local env ready: bucket=${s3__bucketName} at ${s3__endpointUrl}, metadata in ${imageMetadata__parentPath}"
