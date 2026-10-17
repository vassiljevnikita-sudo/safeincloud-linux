"""Replace the entire secret-bearing GUI process on lock.

The supervisor never receives a password, plaintext XML, or a live Vault.
Only ciphertext recovery and nonsecret source metadata cross the pipe.
"""
import json
import os
import subprocess
import sys
from multiprocessing import Pipe
from multiprocessing.connection import Connection

from . import MAX_FILE

RESTART = 73
MAX_STATE = 2 * MAX_FILE + 1024 * 1024


def send_state(connection, state):
    data = json.dumps(state, ensure_ascii=True).encode('ascii')
    if len(data) > MAX_STATE:
        raise ValueError('Session handover exceeds limit')
    connection.send_bytes(data)


def receive_state(connection):
    return json.loads(connection.recv_bytes(MAX_STATE))


def protect_process():
    """Disable core dumps without denying desktop portals /proc access.

    PR_SET_DUMPABLE=0 also blocks GNOME from resolving /proc/<pid>/root,
    breaking portal registration, file selection and system appearance.
    Lock confidentiality relies on process termination, not that flag.
    """
    import resource
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


def source_state(session):
    if session is None:
        return None
    from .drive import DriveSession, GoogleDrive
    from .official_drive import OfficialDrive
    if type(session) is not DriveSession or type(session.drive) not in (GoogleDrive, OfficialDrive):
        raise TypeError('Unsupported session transport')
    return dict(kind='official' if type(session.drive) is OfficialDrive else 'rclone',
                config=str(session.drive.config), candidate=session.candidate,
                etag=session.etag, base_digest=session.base_digest.hex())


def restore_source(state, path):
    if state is None:
        return None
    from .drive import DriveSession, GoogleDrive
    from .official_drive import OfficialDrive
    drive = (OfficialDrive if state['kind'] == 'official' else GoogleDrive)(config=state['config'])
    session = DriveSession(drive, state['candidate'], path, {'etag': state['etag']}, b'')
    session.base_digest = bytes.fromhex(state['base_digest'])
    return session


class SessionBoundary:
    def __init__(self, connection):
        self.connection = connection

    def _finish(self, state, code):
        try:
            send_state(self.connection, state)
        finally:
            # No Python/Qt cleanup, suspended dialog, timer, or network thread
            # may keep the old heap alive after the supervisor receives state.
            os._exit(code)

    def restart(self, state):
        self._finish(state, RESTART)

    def close(self, state):
        self._finish(state, 0)


def save_geometry(state):
    # Only the supervisor (after wait()) performs these fallible disk writes.
    if state and state.get('geometry'):
        import base64
        from PySide6.QtCore import QSettings, QByteArray
        settings = QSettings('SafeInCloudLinux', 'Desktop')
        settings.setValue('geometry', QByteArray(base64.b64decode(state['geometry'])))
        settings.sync()


def launch_isolated(path=None):
    protect_process()
    state = dict(path=str(path) if path is not None else None, source=None, recovery=None)
    while True:
        parent, child = Pipe(duplex=True)
        process = None
        try:
            process = subprocess.Popen(
                [sys.executable, '-m', 'safeincloud_linux.session_process', str(child.fileno())],
                pass_fds=(child.fileno(),))
            child.close()
            send_state(parent, state)
            try:
                next_state = receive_state(parent)
            except EOFError:
                next_state = None
            # A new locked GUI must never coexist with the old secret heap.
            result = process.wait()
        finally:
            parent.close()
            child.close()
            if process is not None and process.poll() is None:
                process.kill()
                process.wait()
        try:
            save_geometry(next_state)
        except Exception:
            # Preferences must never discard encrypted recovery or prevent
            # the fresh locked session from opening after a disk failure.
            pass
        if result != RESTART or next_state is None:
            return result
        state = next_state


def child_main(descriptor):
    protect_process()
    connection = Connection(descriptor)
    state = receive_state(connection)
    from .startup import _launch
    return _launch(state['path'], boundary=SessionBoundary(connection), state=state)


if __name__ == '__main__':
    os._exit(child_main(int(sys.argv[1])) or 0)
