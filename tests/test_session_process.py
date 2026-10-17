"""Real process/Qt lifecycle tests: no real vaults, tokens, or network."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from safeincloud_linux import session_process
from safeincloud_linux.vault import encrypt


@unittest.skipUnless(sys.platform == 'linux', 'Linux desktop process boundary')
class SessionProcessTests(unittest.TestCase):
    def test_lock_replaces_secret_process_including_error_sync_and_dialog_paths(self):
        for mode in ('normal', 'startup', 'failure', 'sync', 'editor', 'blocked-save', 'settings-failure', 'encryption-failure', 'encryption-close'):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                database = Path(directory) / 'synthetic.db'
                database.write_bytes(encrypt(b'<database><card id="1" title="Before"/></database>', 'test-only'))
                processes, states = [], []
                real_popen = subprocess.Popen
                real_receive = session_process.receive_state

                def spawn(arguments, **kwargs):
                    if processes:
                        self.assertEqual(processes[-1].poll(), session_process.RESTART)
                    phase = len(processes)
                    self.assertLess(phase, 2, 'Lock restart loop')
                    probe = Path(__file__).with_name('session_process_probe.py')
                    env = dict(os.environ, QT_QPA_PLATFORM='offscreen',
                               XDG_CONFIG_HOME=str(Path(directory) / 'config'),
                               PYTHONPATH=str(Path(session_process.__file__).parent.parent))
                    process = real_popen([sys.executable, str(probe), arguments[-1], mode, str(phase)],
                                         env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, **kwargs)
                    processes.append(process)
                    return process

                def receive(connection):
                    state = real_receive(connection)
                    states.append(state)
                    text = json.dumps(state)
                    self.assertNotIn('test-only', text)
                    self.assertNotIn('unsaved-demo', text)
                    return state

                def persist_geometry(state):
                    self.assertIsNotNone(processes[-1].poll(), 'Settings I/O preceded process exit')
                    if mode == 'settings-failure':
                        raise OSError('synthetic settings disk failure')

                # Protect only the real children; don't disable dumps on the
                # unittest host or change its inherited resource limits.
                with patch.object(session_process.subprocess, 'Popen', side_effect=spawn), \
                     patch.object(session_process, 'protect_process'), \
                     patch.object(session_process, 'save_geometry', side_effect=persist_geometry), \
                     patch.object(session_process, 'receive_state', side_effect=receive):
                    self.assertEqual(session_process.launch_isolated(database), 0)
                self.assertEqual(len(processes), 2)
                self.assertEqual(len(states), 2)
                self.assertNotEqual(processes[0].pid, processes[1].pid)
                for index, process in enumerate(processes):
                    output, errors = process.communicate(timeout=2)
                    if index == 1:
                        result = json.loads(output)
                        self.assertTrue(result['restored'], errors.decode())
                        self.assertEqual(result['dumpable'], 0)
                        self.assertEqual(result['core_limit'], 0)

    def test_source_handover_keeps_only_nonsecret_metadata(self):
        from safeincloud_linux.drive import DriveSession
        from safeincloud_linux.official_drive import OfficialDrive
        drive = OfficialDrive(config='/synthetic/rclone.conf')
        session = DriveSession(drive, {'id': 'synthetic', 'path': 'SafeInCloud.db'},
                               '/synthetic/vault.db', {'etag': '"base"'}, b'ciphertext')
        state = session_process.source_state(session)
        self.assertEqual(set(state), {'kind', 'config', 'candidate', 'etag', 'base_digest'})
        restored = session_process.restore_source(state, session.path)
        self.assertIsInstance(restored.drive, OfficialDrive)
        self.assertEqual(restored.etag, session.etag)
        self.assertEqual(restored.base_digest, session.base_digest)
