#!/bin/bash
# Download mainnet state diffs from the public Xatu dataset (ethpandaops,
# CC BY 4.0, no key). One parquet file per table per 1000 blocks.
#
# usage: tools/mainnet/fetch.sh OUT_DIR START_BLOCK N_CHUNKS [reads]
#   START_BLOCK must be a multiple of 1000. About 55 MB per 1000 blocks.
#   With "reads", also the execution reads (storage, balance, nonce):
#   about 72 MB more per 1000 blocks.
#
# Keep OUT_DIR outside the repository: the data is large and reproducible
# from START_BLOCK and N_CHUNKS, which are what a report has to record.
set -euo pipefail
OUT="$1"; START="$2"; N="$3"; EXTRA="${4:-}"
BASE="https://data.ethpandaops.io/xatu/mainnet/databases/default"
TABLES="storage_diffs balance_diffs nonce_diffs contracts"
[ "$EXTRA" = reads ] && TABLES="$TABLES storage_reads balance_reads nonce_reads"
for t in $TABLES; do
  mkdir -p "$OUT/$t"
  for ((i = 0; i < N; i++)); do
    c=$((START + i * 1000))
    f="$OUT/$t/$c.parquet"
    if [ ! -s "$f" ]; then
      curl -sf --retry 3 -o "$f" "$BASE/canonical_execution_$t/1000/$c.parquet" || {
        echo "FAILED $t $c" >&2
        exit 1
      }
    fi
  done
done
du -sh "$OUT" | cut -f1
