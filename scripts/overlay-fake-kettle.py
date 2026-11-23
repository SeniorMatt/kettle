#!/usr/bin/env python3
"""A stand-in for Kettle's side of the overlay socket, for scripts/overlay-kettle-test.sh.

Listens on the socket path in argv[1], sends every library that connects the data Kettle sends (playtime,
achievements, friends, a picture for the account and for two friends, a screenshot folder) and appends every
line it receives to the file in argv[2]. Stops when the file argv[3] appears.
"""
import base64
import json
import os
import socket
import sys
import threading
import time


def square(size, rgb):
    px = bytes(rgb + (255,)) * (size * size)
    return base64.b64encode(px).decode()


def main():
    path, log, quit_file = sys.argv[1:4]
    shots = os.environ.get("FAKE_SHOT_DIR", "/tmp/kettle-fake-shots")
    now = int(time.time())
    data = {
        "t": "data", "app": 620, "game": "Fake Game", "session_secs": 3000, "me": "Kettle Tester",
        "me_id": 1, "friends_connected": True, "total_minutes": 2304, "screenshot_dir": shots,
        "friends": [
            {"id": 11, "name": "Red Friend", "status": "Fake Game", "online": True, "in_game": True},
            {"id": 12, "name": "Green Friend", "status": "Online", "online": True, "in_game": False},
            {"id": 13, "name": "Away Friend", "status": "Away", "online": True, "in_game": False},
        ],
        "achievements_unlocked": 9, "achievements_total": 63,
        "achievements_recent": [
            {"name": "Honour", "description": "Defeat the Dung Defender", "time": now - 600},
            {"name": "Release", "description": "Defeat the Broken Vessel", "time": now - 4000},
        ],
        "hud": False, "enabled": True, "animations": False,
    }
    pics = {1: (40, 90, 200), 11: (220, 40, 40), 12: (40, 200, 60)}
    srv = socket.socket(socket.AF_UNIX)
    if os.path.exists(path):
        os.unlink(path)
    srv.bind(path)
    srv.listen(4)
    srv.settimeout(0.3)

    def serve(conn):
        conn.sendall((json.dumps(data) + "\n").encode())
        for i, rgb in pics.items():
            msg = {"t": "avatar", "id": i, "size": 48, "rgba": square(48, rgb)}
            conn.sendall((json.dumps(msg) + "\n").encode())
        with conn, open(log, "a") as out:
            f = conn.makefile("r")
            for line in f:
                out.write(line)
                out.flush()

    while not os.path.exists(quit_file):
        try:
            conn, _ = srv.accept()
        except socket.timeout:
            continue
        threading.Thread(target=serve, args=(conn,), daemon=True).start()
    srv.close()
    os.unlink(path)


main()
