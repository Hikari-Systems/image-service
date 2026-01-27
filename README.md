# Image Service

A robust, production-ready image management service built with Node.js, TypeScript, and Express. This service handles image uploads, processing, storage, and delivery through AWS S3 and CloudFront.

## Features

- **Image Upload**: Upload images via multipart/form-data with category-based organization
- **Image Processing**: Automatic resizing and transcoding using ImageMagick
- **Multiple Sizes**: Configurable image size presets (small, medium, large, etc.)
- **Cloud Storage**: Integration with AWS S3 for reliable image storage
- **CDN Delivery**: CloudFront integration with signed URLs for secure image delivery
- **Metadata Management**: JSON-based metadata storage for image information
- **Category System**: Organize images by category with custom scaling sets
- **Deferred Processing**: Optional deferred transcoding for improved performance
- **SVG Sanitization**: Built-in SVG sanitization for security

## Prerequisites

- Node.js 22 or higher
- ImageMagick (built from source in Docker, or install system-wide)
- AWS S3 bucket and credentials
- AWS CloudFront distribution (optional, for CDN delivery)
- TypeScript 5.8+

## Installation

1. Clone the repository:
```bash
git clone <repository-url>
cd image-service
```

2. Install dependencies:
```bash
npm install
```

3. Configure the service by editing `config.json` (see Configuration section)

4. Build the project:
```bash
npm run build
```

5. Start the service:
```bash
npm start
```

## Configuration

The service is configured via `config.json`. Here's a sample configuration:

```json
{
  "server": {
    "port": 3000
  },
  "log": {
    "level": "debug"
  },
  "imageMetadata": {
    "parentPath": "/metadata"
  },
  "imagemagick": {
    "bin": "/usr/bin/magick"
  },
  "uploadDir": "/tmp",
  "s3": {
    "bucketName": "your-bucket-name",
    "accessKeyId": "your-access-key",
    "secretAccessKey": "your-secret-key"
  },
  "cloudfront": {
    "url": "https://your-cloudfront-domain.cloudfront.net",
    "expirySeconds": "10100",
    "keypairId": "your-keypair-id",
    "privateKey": "your-private-key",
    "privateKeyFile": "/path/to/private-key.pem"
  },
  "resize": {
    "processing": "deferred",
    "sizeKeys": "small,medium,large",
    "original": {
      "mimeType": "image/png",
      "extension": ".png"
    },
    "small": {
      "width": 100,
      "height": 100,
      "mimeType": "image/jpg",
      "extension": ".jpg"
    },
    "medium": {
      "width": 200,
      "height": 200,
      "mimeType": "image/jpg",
      "extension": ".jpg"
    },
    "large": {
      "width": 400,
      "height": 400,
      "mimeType": "image/png",
      "extension": ".png"
    }
  }
}
```

### Configuration Options

- **server.port**: Port number for the HTTP server (default: 3000)
- **log.level**: Logging level (debug, info, warn, error)
- **imageMetadata.parentPath**: Directory path for storing image metadata JSON files
- **imagemagick.bin**: Path to ImageMagick binary
- **uploadDir**: Temporary directory for uploaded files
- **s3**: AWS S3 configuration (bucket name and credentials)
- **cloudfront**: CloudFront CDN configuration for signed URLs
- **resize**: Image resizing configuration
  - **processing**: "deferred" or "immediate"
  - **sizeKeys**: Comma-separated list of default size presets
  - **{sizeName}**: Individual size configuration with width, height, mimeType, and extension

### Custom Categories

You can define custom scaling sets for categories by adding them to the config:

```json
{
  "resize": {
    "scalingSets": {
      "thumbnail": "small,medium",
      "gallery": "small,medium,large,original"
    }
  }
}
```

## API Endpoints

### Health Check

```http
GET /healthcheck
```

Returns `200 OK` if the service is running.

### Upload Image

```http
POST /api/image/:category
Content-Type: multipart/form-data
```

Upload an image to a specific category.

**Parameters:**

- `category` (path): Category name for organizing the image
- `image` (form-data): Image file to upload
- `forceImmediateResize` (query, optional): Set to `true` to process immediately instead of deferred

**Response:** `201 Created` with image metadata JSON

**Example:**

```bash
curl -X POST \
  http://localhost:3000/api/image/products?forceImmediateResize=true \
  -F "image=@/path/to/image.jpg"
```

### Get Image by ID

```http
GET /api/image/:id
```

Retrieve image metadata by ID.

**Response:** `200 OK` with image metadata JSON, or `404 Not Found`

### Get Resized Image (Redirect)

```http
GET /api/image/r/:id/:size
```

Get a resized image URL. Returns a redirect to the CloudFront signed URL.

**Parameters:**

- `id`: Image ID
- `size`: Size preset name (e.g., "small", "medium", "large")

**Response:** `302 Redirect` to the image URL, or `404 Not Found`

### Get Resized Image URL (JSON)

```http
GET /api/image/s/:id/:size
```

Get a resized image URL as JSON.

**Response:** `200 OK` with JSON `{ "url": "https://..." }`, or `404 Not Found`

### Transcode Image

```http
POST /api/image/:id/transcode
```

Trigger transcoding for an image that has been uploaded but not yet processed.

**Response:** `200 OK` with transcoded image metadata, or `404 Not Found`

### List Categories

```http
GET /api/category/list
```

Get a list of all available categories and their size configurations.

**Response:** `200 OK` with JSON array of categories and their sizes

## Development

### Scripts

- `npm run build`: Compile TypeScript to JavaScript
- `npm run build:esbuild`: Bundle the application with esbuild
- `npm run watch`: Watch mode for TypeScript compilation
- `npm run lint`: Run ESLint
- `npm test`: Run tests
- `npm run testci`: Run tests in CI mode

### Project Structure

```bash
image-service/
├── lib/
│   ├── helpers/          # Helper functions (transcoding, multer)
│   ├── model/            # Data models (image metadata)
│   ├── route/            # Express route handlers
│   ├── service/          # External service integrations (S3, CloudFront, ImageMagick)
│   ├── types/            # TypeScript type definitions
│   ├── index.ts          # Route aggregator
│   └── server.ts         # Express server setup
├── __tests__/            # Test files
├── static/               # Static files
├── config.json           # Configuration file
├── Dockerfile            # Docker build configuration
└── docker-compose.yml    # Docker Compose configuration
```

## Docker

### Building the Image

The Dockerfile builds ImageMagick from source and creates an optimized production image:

```bash
docker build -t image-service .
```

### Running with Docker Compose

```bash
docker-compose up
```

The service will be available at `http://localhost:3001` (mapped from container port 3000).

### Docker Configuration

The `docker-compose.yml` includes:

- Read-only root filesystem for security
- Volume mounts for temporary files and metadata
- Port mapping (3001:3000)

## Security Considerations

- The service runs as a non-root user (`nobody`) in Docker
- SVG files are sanitized before processing
- CloudFront signed URLs provide time-limited access to images
- Temporary upload files are automatically cleaned up

## Dependencies

### Core Dependencies

- **express**: Web framework
- **@aws-sdk/client-s3**: AWS S3 client
- **@aws-sdk/cloudfront-signer**: CloudFront URL signing
- **multer**: File upload handling
- **dompurify**: SVG sanitization
- **uuid**: Unique ID generation

### Development Dependencies

- **typescript**: TypeScript compiler
- **eslint**: Code linting
- **prettier**: Code formatting
- **jest**: Testing framework
- **esbuild**: Fast bundler

## Environment Variables

The service uses the `@hikari-systems/hs.utils` package for configuration management. Ensure your environment is properly configured to load settings from `config.json` or environment variables as needed.

## Contributing

Contributions are welcome! Please follow these guidelines:

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Make your changes
4. Run tests and linting (`npm test && npm run lint`)
5. Commit your changes (`git commit -m 'Add some amazing feature'`)
6. Push to the branch (`git push origin feature/amazing-feature`)
7. Open a Pull Request

## License

This project is licensed under the Apache License 2.0. See the [LICENSE](LICENSE) file for details.

For more information about the Apache License 2.0, visit [https://www.apache.org/licenses/LICENSE-2.0](https://www.apache.org/licenses/LICENSE-2.0).

## Support

For issues, questions, or contributions, please open an issue on the GitHub repository.
