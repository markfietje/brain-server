#!/bin/sh
# v1.28.50 "Aqueduct" local eval gate — mirrors ci.yml's recall-eval job:
# seed a scratch instance with the frozen 25-doc corpus, then run
# `brain eval --floor r5=0.85 --floor r10=0.85 --floor mrr=0.85` against it.
# Usage: scripts/aqueduct-eval.sh <port>
set -e
PORT="${1:-18484}"
SCRATCH="$(mktemp -d /tmp/aqueduct-eval.XXXXXX)"
echo "$SCRATCH" > /tmp/aqueduct-eval-scratch
python3 - "$SCRATCH" <<'EOF'
import sys, pathlib
docs = [
  "Bignay is a tropical fruit and a good alternative to blueberry, rich in antioxidants.",
  "The Rust programming language guarantees memory safety without a garbage collector.",
  "Vitamin D3 supplementation improves immune function and bone density in deficient adults.",
  "The GDPR is a European regulation protecting the personal data of EU residents.",
  "Gut microbiome diversity affects inflammation markers and immune system regulation.",
  "SQLite is an embedded relational database with FTS5 full-text search support.",
  "ISO 9001 is the international standard for quality management systems.",
  "Ownership and borrowing are Rust's core concepts for compile-time memory safety.",
  "Antioxidants in tropical fruits like bignay help reduce oxidative stress.",
  "The GDPR covers any organization processing EU residents' data, with fines up to four percent of global revenue.",
  # migration vertical (docs 10-14)
  "VxRail LCM upgrades require a green RCM release certification manifest before any upgrade wave is scheduled.",
  "A stretched-cluster rolling reboot reboots one ESXi node at a time; never reboot two nodes concurrently.",
  "vSAN storage policies set FTT failures to tolerate and FTM failure tolerance method per virtual machine.",
  "PowerFlex protection domains map fault sets to failure boundaries across SDS storage pools.",
  "NSX-T managers push micro-segmentation firewall rules to transport nodes over the control plane.",
  # legal vertical (docs 15-19)
  "A DPA data processing agreement under GDPR Article 28 binds the processor to the controller's instructions.",
  "Standard Contractual Clauses 2021 are the approved EU transfer mechanism for processors outside the EEA.",
  "RA 10173 the Philippine Data Privacy Act requires NPC breach notification within 72 hours.",
  "Schrems II requires a transfer impact assessment before any personal-data transfer to a third country.",
  "Legal holds freeze erasure until every hold is explicitly released by the operator.",
  # troubleshoot vertical (docs 20-24)
  "Intermittent storage fabric latency usually traces to a failing SFP on one uplink port, not the array.",
  "High VM disk latency triage order: vSAN backend congestion, then host cache, then the physical disk group.",
  "A node flapping out of vCenter management is most often NTP drift breaking certificate validation.",
  "PSOD purple diagnostic screen dumps land in var log and must be collected before any reboot clears them.",
  "vMotion failing at ten percent points to VMkernel port mobility or a missing shared datastore.",
]
d = pathlib.Path(sys.argv[1])
(d / "corpus").mkdir()
for i, doc in enumerate(docs):
    (d / "corpus" / f"doc{i}.md").write_text(f"---\ntitle: doc{i}\n---\n\n{doc}\n")
EOF
BIND_PORT="$PORT" BRAIN_DB_PATH="$SCRATCH/brain.db" target/release/brain-server >"$SCRATCH/server.log" 2>&1 &
echo $! > /tmp/aqueduct-eval-pid
i=0
while [ "$i" -lt 60 ]; do
  if curl -fsS "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then break; fi
  sleep 1
  i=$((i+1))
done
curl -fsS "http://127.0.0.1:$PORT/health" >/dev/null
OUT="$(BRAIN_URL="http://127.0.0.1:$PORT" target/release/brain ingest-dir "$SCRATCH/corpus")"
echo "$OUT"
case "$OUT" in
  *"25 ingested"*) ;;
  *) echo "seed ingest failed (expected '25 ingested')" && kill "$(cat /tmp/aqueduct-eval-pid)" 2>/dev/null || true && exit 1 ;;
esac
BRAIN_URL="http://127.0.0.1:$PORT" target/release/brain eval \
  --floor r5=0.85 --floor r10=0.85 --floor mrr=0.85 2>&1 | tail -6
STATUS=$?
kill "$(cat /tmp/aqueduct-eval-pid)" 2>/dev/null || true
exit "$STATUS"
