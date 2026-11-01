#!/usr/bin/env bash
# End-to-end check of the variant-selection parameters against MinIO.
#
#   docker compose up -d minio minio-init
#   source scripts/local-env.sh && cargo run &
#   ./scripts/local-smoke-test.sh
#
# Asserts on the record the API returns and on what actually landed in the bucket.
set -uo pipefail

BASE="${BASE:-http://localhost:3000}"
S3="${S3:-http://localhost:9000/image-service-local}"
IMG="$(mktemp -d)/photo.jpg"
FAILURES=0

magick_bin=$(command -v magick || command -v convert)
"$magick_bin" -size 600x400 gradient:steelblue-orange "$IMG"

# Variants present on a record, as a sorted space-separated list. "original" is
# reported when originalS3Path is set, so one list covers every variant.
variants() {
  python3 -c '
import json,sys
r = json.load(sys.stdin)
v = ["original"] if r.get("originalS3Path") else []
v += [f["size"] for f in (r.get("resizedFiles") or [])]
print(" ".join(sorted(v)))'
}

record_id() { python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])'; }

check() { # check <label> <expected> <actual>
  if [ "$2" = "$3" ]; then
    echo "  ok   $1: [$3]"
  else
    echo "  FAIL $1: expected [$2] got [$3]"
    FAILURES=$((FAILURES + 1))
  fi
}

upload() { curl -s -F "image=@$IMG" "$BASE/api/image/test$1"; }

echo "== upload variant selection"
check "?forceImmediateResize=true" "large medium original small" \
      "$(upload '?forceImmediateResize=true' | variants)"
check "?forceImmediateResize=original,small" "original small" \
      "$(upload '?forceImmediateResize=original,small' | variants)"
check "?forceImmediateResize=small,large" "large small" \
      "$(upload '?forceImmediateResize=small,large' | variants)"
check "?forceImmediateResize=true&defer=large" "medium original small" \
      "$(upload '?forceImmediateResize=true&defer=large' | variants)"
check "?defer=original (deferred config)" "" \
      "$(upload '?defer=original' | variants)"
check "?forceImmediateResize=small&defer=small" "" \
      "$(upload '?forceImmediateResize=small&defer=small' | variants)"
check "no params (deferred config)" "" "$(upload '' | variants)"

echo "== rejected keys"
for q in 'forceImmediateResize=nosuchsize' 'defer=nosuchsize'; do
  code=$(curl -s -o /dev/null -w '%{http_code}' -F "image=@$IMG" "$BASE/api/image/test?$q")
  check "?$q" "400" "$code"
done

echo "== transcode endpoint completes an upload without losing earlier work"
id=$(upload '?forceImmediateResize=original,small' | record_id)
# GET returns the record flattened alongside originalFileUrl, so it feeds `variants`
# directly — and reading it back proves the subset was persisted, not just returned.
check "after upload" "original small" "$(curl -s "$BASE/api/image/$id" | variants)"
check "after ?sizes=medium" "medium original small" \
      "$(curl -s -X POST "$BASE/api/image/$id/transcode?sizes=medium" | variants)"
check "after ?defer=small" "large medium original small" \
      "$(curl -s -X POST "$BASE/api/image/$id/transcode?defer=small" | variants)"

echo "== objects actually in the bucket"
for suffix in ".jpg" "-original.jpg" "-small.jpg" "-medium.jpg" "-large.png"; do
  code=$(curl -s -o /dev/null -w '%{http_code}' "$S3/test-$id$suffix")
  check "test-$id$suffix" "200" "$code"
done

echo "== a variant nobody asked for was never written"
id2=$(upload '?forceImmediateResize=small' | record_id)
code=$(curl -s -o /dev/null -w '%{http_code}' "$S3/test-$id2-large.png")
check "test-$id2-large.png absent" "404" "$code"

echo
if [ "$FAILURES" -eq 0 ]; then echo "all checks passed"; else echo "$FAILURES check(s) failed"; fi
exit "$FAILURES"
