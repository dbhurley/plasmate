#!/bin/bash
# Legacy public-web serialized-byte observation helper
# This does not measure tokens, model cost, task quality, or comparable engines.
# Run: ./benchmarks/run-cost-analysis.sh
# Requires: plasmate binary in PATH or target/release/plasmate

set -e

PLASMATE="${PLASMATE_BIN:-$(command -v plasmate 2>/dev/null || echo target/release/plasmate)}"
if [ ! -x "$PLASMATE" ]; then
  echo "Error: plasmate binary not found. Install with: cargo install plasmate"
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
URLS_FILE="$SCRIPT_DIR/urls.txt"
OUTPUT="$SCRIPT_DIR/results-$(date +%Y-%m-%d).json"

echo "SOM Serialized-Byte Observation"
echo "Plasmate: $($PLASMATE --version 2>/dev/null || echo 'unknown')"
echo "Date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "URLs: $(wc -l < "$URLS_FILE" | tr -d ' ')"
echo "Output: $OUTPUT"
echo ""

echo '[]' > "$OUTPUT"
total=0
success=0

while IFS= read -r url; do
  [ -z "$url" ] && continue
  [[ "$url" == \#* ]] && continue
  total=$((total + 1))

  result=$("$PLASMATE" fetch "$url" 2>/dev/null | python3 -c "
import sys,json
try:
  d=json.load(sys.stdin)
  m=d.get('meta',{})
  html=m.get('html_bytes',0)
  som=m.get('som_bytes',0)
  ratio=html/max(som,1)
  print(json.dumps({
    'url': sys.argv[1],
    'html_bytes': html,
    'som_bytes': som,
    'serialized_byte_ratio': round(ratio,1),
    'elements': m.get('element_count',0),
    'interactive': m.get('interactive_count',0)
  }))
except:
  print(json.dumps({'url': sys.argv[1], 'error': 'fetch_failed'}))
" "$url" 2>/dev/null)

  if echo "$result" | python3 -c "import sys,json; d=json.load(sys.stdin); exit(0 if 'error' not in d else 1)" 2>/dev/null; then
    ratio=$(echo "$result" | python3 -c "import sys,json; print(json.load(sys.stdin)['serialized_byte_ratio'])")
    echo "  OK  ${ratio}x  $url"
    success=$((success + 1))
  else
    echo "  FAIL     $url"
  fi

  python3 -c "
import json, sys
with open(sys.argv[1],'r') as f: data=json.load(f)
data.append(json.loads(sys.argv[2]))
with open(sys.argv[1],'w') as f: json.dump(data,f,indent=2)
" "$OUTPUT" "$result"

done < "$URLS_FILE"

echo ""
echo "Done: $success/$total succeeded"
echo ""

# Print summary
python3 -c "
import json, sys
with open(sys.argv[1]) as f: data = json.load(f)
valid = [d for d in data if 'error' not in d]
if not valid:
    print('No valid results')
    sys.exit(1)

total_html = sum(d['html_bytes'] for d in valid)
total_som = sum(d['som_bytes'] for d in valid)
ratios = sorted([d['serialized_byte_ratio'] for d in valid])
median = ratios[len(ratios)//2]

print(f'Attempted inputs:     {len(data)}')
print(f'Successful inputs:    {len(valid)}')
print(f'Failed inputs:        {len(data) - len(valid)}')
print(f'Total HTML bytes:     {total_html:,}')
print(f'Total SOM bytes:      {total_som:,}')
print(f'Aggregate byte ratio: {total_html/total_som:.1f}x')
print(f'Median byte ratio:    {median:.1f}x')
print('These are workload-specific serialized-byte observations, not token, cost, latency, or task-success claims.')
" "$OUTPUT"
