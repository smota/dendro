"""Dedicated synthetic subprocess fixture; never a Dendro performance candidate."""
import json
import os
import sys
import time

behavior = sys.argv[1]
if behavior == "fail":
    print('{"status":"ok"}')
    sys.exit(17)
if behavior == "timeout":
    time.sleep(10)
elif behavior == "flood":
    for _ in range(10000):
        os.write(1, b"x" * 4096)
elif behavior == "stderr":
    os.write(2, b"x" * 4096)
elif behavior == "signal":
    os._exit(9)
elif behavior == "answer":
    req = json.loads(sys.argv[2])
    print(json.dumps(dict(req, status="ok", record_count=82,
                          answer_count=7 if req['mode'] == 'seven_answer_home' else 1)))
else:
    print("synthetic-ok")
