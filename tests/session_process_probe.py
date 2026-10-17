"""Subprocess-only synthetic integration fixture; never used by the application."""
import ctypes
import json
import os
import resource
import sys
import threading
from unittest.mock import patch

from PySide6.QtCore import QTimer
from PySide6.QtWidgets import QApplication
from safeincloud_linux import startup
from safeincloud_linux.qt_ui import Window, Editor
from safeincloud_linux.session_process import child_main

original_launch = startup._launch


def real_startup_probe(path, *, boundary, state):
    app = QApplication([])
    def act():
        window = next(w for w in app.topLevelWidgets() if isinstance(w, Window))
        assert window.boundary is boundary and window.locked and window.vault is None
        assert not window.unlock('wrong')
        assert window.unlock('test-only')
        if int(sys.argv[3]) == 0:
            window.lock()
        else:
            print(json.dumps(dict(restored=True, pid=os.getpid(),
                                  dumpable=ctypes.CDLL(None).prctl(3, 0, 0, 0, 0),
                                  core_limit=resource.getrlimit(resource.RLIMIT_CORE)[0])), flush=True)
            window.close()
    QTimer.singleShot(8000, lambda: os._exit(91))
    QTimer.singleShot(0, act)
    return original_launch(path, boundary=boundary, state=state)


def probe_launch(path, *, boundary, state):
    if sys.argv[2] == 'startup':
        return real_startup_probe(path, boundary=boundary, state=state)
    app = QApplication([])
    window = Window(path, settings=False, boundary=boundary, recovery=state.get('recovery'))
    window.show()
    mode, phase = sys.argv[2], int(sys.argv[3])
    QTimer.singleShot(8000, lambda: os._exit(91))

    def act():
        if phase == 0:
            assert window.unlock('test-only')
            if mode in ('failure', 'encryption-failure', 'editor', 'blocked-save', 'encryption-close', 'settings-failure'):
                window.vault.upsert('1', 'Pending', [dict(name='Password', type='password', value='unsaved-demo')])
            if mode == 'sync':
                started = threading.Event()
                def blocked(vault):
                    assert vault.password == 'test-only'
                    started.set()
                    threading.Event().wait()  # Lock must kill this whole thread.
                window.on_sync = blocked
                window.sync()
                assert started.wait(2)
                window.lock()
            elif mode in ('failure', 'settings-failure'):
                with patch.object(window.vault, 'save', side_effect=OSError('synthetic full disk')):
                    window.lock()
            elif mode == 'blocked-save':
                def blocked_save():
                    threading.Event().wait()
                with patch.object(window.vault, 'save', side_effect=blocked_save):
                    window.lock()
            elif mode in ('encryption-failure', 'encryption-close'):
                with patch.object(window.vault, 'serialize_encrypted', side_effect=RuntimeError('synthetic failure')):
                    window.close() if mode == 'encryption-close' else window.lock()
            elif mode == 'editor':
                QTimer.singleShot(20, window.lock)
                window.edit_entry()
            else:
                window.lock()
            raise AssertionError('Session boundary returned')
        assert window.vault is None and window.locked
        assert not window.unlock('wrong')
        expected_recovery = mode in ('failure', 'blocked-save', 'editor', 'settings-failure')
        assert (window.recovery is not None) == expected_recovery
        if mode in ('encryption-failure', 'encryption-close'):
            assert 'verworfen' in state['notice']
        assert window.unlock('test-only')
        expected = 'Pending' if mode in ('failure', 'blocked-save', 'editor', 'settings-failure') else 'Before'
        assert window.current_card().get('title') == expected
        if window.vault.dirty:
            assert window.persist()
        print(json.dumps(dict(restored=True, pid=os.getpid(),
                              dumpable=ctypes.CDLL(None).prctl(3, 0, 0, 0, 0),
                              core_limit=resource.getrlimit(resource.RLIMIT_CORE)[0])), flush=True)
        window.close()

    QTimer.singleShot(0, act)
    return app.exec()


startup._launch = probe_launch
os._exit(child_main(int(sys.argv[1])) or 0)
