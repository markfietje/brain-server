#!/usr/bin/env python3
"""Live smoke: Switchboard channel seams against a real brain-server on a DB COPY.

Proves (execution prompt order):
  1. bridge boots + registers mount evidence (server-side config-digest verify)
  2. inbound lands as a screened note under the bridge's domain
  3. unknown conversation auto-opens a care/case run under that domain
  4. [case N] addressing overrides the thread map
  5. replayed webhook is a no-op (duplicate)
  6. outbound drain returns nothing without an approved act
  7. /audit/verify ok
"""
import base64, hashlib, hmac, json, os, shutil, sqlite3, sys, tempfile, time, urllib.request

PASS = []
def ok(name, cond, extra=""):
    PASS.append((name, bool(cond)))
    print(("✅" if cond else "❌"), name, extra)
    if not cond:
        pass  # keep running to collect full picture; exit code decides

def sign_headers(secret: bytes, body: bytes):
    wid = f"smoke-{time.time_ns()}"
    ts = str(int(time.time()))
    mac = hmac.new(secret, f"{wid}.{ts}.".encode() + body, hashlib.sha256)
    sig = "v1," + base64.b64encode(mac.digest()).decode()
    return {"webhook-id": wid, "webhook-timestamp": ts, "webhook-signature": sig}

def post(url, secret: bytes, body_obj):
    body = json.dumps(body_obj).encode()
    req = urllib.request.Request(url, data=body, method="POST")
    hdrs = sign_headers(secret, body)
    for k, v in hdrs.items():
        req.add_header(k, v)
    req.add_header("content-type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            return r.status, json.loads(r.read().decode())
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()[:200]

def get_json(url):
    with urllib.request.urlopen(url, timeout=10) as r:
        return json.loads(r.read().decode())

# ── environment ──
work = tempfile.mkdtemp(prefix="switchboard-smoke-")
db_src = os.path.expanduser("~/.openclaw/workspace/brain.db")
db_dst = os.path.join(work, "brain.db")
copy_note = "copied LIVE DB"
try:
    shutil.copy(db_src, db_dst)          # DB COPY per prompt law
except FileNotFoundError:
    open(db_dst, "wb").close()           # absent live db → fresh schema via migration
    copy_note = "fresh db (no live brain.db found)"

conn_dir = os.path.join(work, "connectors"); os.makedirs(conn_dir, mode=0o700, exist_ok=True)
cfg_path = os.path.join(conn_dir, "channel-signal-owner.json")
SECRET = b"smoke-bridge-secret-0123456789abcdef"
cfg_body = json.dumps({"domain": "global", "webhook_secret": SECRET.decode()})
open(cfg_path, "w").write(cfg_body)
os.chmod(cfg_path, 0o600)

port = 18765
env = dict(os.environ,
           BRAIN_DB_PATH=db_dst,
           BRAIN_CONNECTOR_CONFIG_DIR=conn_dir,
           BIND_PORT=str(port))
import subprocess
proc = subprocess.Popen(["/Users/mark/Sites/brain-server/target/debug/brain-server"],
                        env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
base = f"http://127.0.0.1:{port}"
for _ in range(60):
    time.sleep(0.5)
    try:
        get_json(base + "/health"); break
    except Exception:
        continue
else:
    print("❌ server never became healthy"); proc.kill(); sys.exit(1)

try:
    # 1 ── mount registration with config digest
    digest = hashlib.sha256(open(cfg_path,'rb').read()).hexdigest()
    st, resp = post(base + "/workflow/plugins/mount", SECRET, {
        "plugin": "channel:signal", "action": "mount",
        "domain": "global", "bundle_sha256": digest})
    ok("mount registered (config-hash digest verified)", st == 200,
       f"status={st} resp={str(resp)[:160]}")

    # 2+3 ── inbound from unknown conversation → auto-open case in bridge domain
    st, r = post(base + "/webhooks/channel/signal", SECRET, {"envelope": {
        "conversation_ref": "+639171234567", "text": "hello from smoke", "external_id": "m-1"}})
    opened = st == 200 and isinstance(r, dict) and r.get("opened_case") \
              and r.get("status") == "note_recorded"
    run_id = r.get("case_run_id") if isinstance(r, dict) else None
    ok("inbound lands + auto-opens care/case", opened, f"run={run_id}")

    def q(sql, args=()):
        c = sqlite3.connect(f"file:{db_dst}?mode=ro", uri=True, timeout=5)
        try:
            return c.execute(sql, args).fetchone()
        finally:
            c.close()

    if run_id:
        kind_dom = q("select kind,domain from workflow_runs where id=?", (run_id,))
        ok("kind==care/case & domain==global",
           bool(kind_dom) and kind_dom[0] == "care/case" and kind_dom[1] == "global")
    else:
        ok("case row present", False)

    n = q("select count(*) from case_notes where run_id=? and content like '%hello from smoke%'",
          (run_id,))[0] if run_id else 0
    ok("screened note stored on the case", n == 1)

    t = q("select count(*) from channel_threads where case_run_id=? and tenant='owner'",
          (run_id,))[0] if run_id else 0
    ok("thread map row created (tenant-scoped)", t == 1)

    # 4 ── [case N] override from a DIFFERENT conversation
    st, r2 = post(base + "/webhooks/channel/signal", SECRET, {"envelope": {
        "conversation_ref": "+639171112222",
        "text": f"[case {run_id}] steering note from smoke",
        "external_id": "m-2"}})
    landed = (st == 200 and isinstance(r2, dict) and r2.get("status") == "note_recorded"
              and r2.get("case_run_id") == run_id and not r2.get("opened_case"))
    ok("[case N] override threads without map entry", landed, f"{st} {str(r2)[:120]}")

    # cross-domain refusal: address a run OUTSIDE bridge domain (write via a
    # dedicated connection AFTER pausing nothing — server tolerates writers).
    x_conn = sqlite3.connect(db_dst, timeout=10)
    x_conn.execute(
        "insert into workflow_runs(domain,kind,state_json,status,created_at,updated_at)"
        " values ('other-domain','intake','{}','active',1,1)")
    x_conn.commit()
    foreign = x_conn.execute("select last_insert_rowid()").fetchone()[0]
    x_conn.close()
    st3, _r3 = post(base + "/webhooks/channel/signal", SECRET, {"envelope": {
        "conversation_ref": "+639171234567",
        "text": f"[case {foreign}] crossing",
        "external_id": "m-3"}})
    ok("cross-domain [case N] refused (409)", st3 == 409)

    # 5 ── replay no-op (same external_id, fresh signature/id)
    st, rr = post(base + "/webhooks/channel/signal", SECRET, {"envelope": {
        "conversation_ref": "+639171234567", "text": "hello from smoke", "external_id": "m-1"}})
    ok("replayed webhook is a no-op", st == 200 and rr.get("status") == "duplicate")

    # 6 ── outbound requires approved act (drain empty w/o one)
    st, dr = post(base + "/webhooks/channel/signal/drain", SECRET, {})
    ok("drain empty absent approved act", st == 200 and dr.get("count") == 0)

    # 7 ── audit chain verifies
    v = get_json(base + "/audit/verify")
    ok("/audit/verify ok", v.get("ok") is True or v.get("ok") == "true")

finally:
    proc.terminate()
    try: proc.wait(timeout=5)
    except Exception: proc.kill()

print(f"\n[{copy_note}] workdir {work}")
bad = [n for n, c in PASS if not c]
print(f"\n{len(PASS)-len(bad)}/{len(PASS)} smoke checks passed")
sys.exit(1 if bad else 0)
