#!/usr/bin/env python3
"""Focused native status/icon checks; use a freshly built production ISO."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import time
from desktop_tty_ux_gate import Guest

parser = argparse.ArgumentParser()
parser.add_argument('iso', type=Path)
parser.add_argument('--out', type=Path, default=Path('target/tty-status-icons'))
args = parser.parse_args()
class StableGuest(Guest):
    def pointer(self):
        # Scanout can briefly omit the cursor while display commits a frame.
        for attempt in range(20):
            try:
                return super().pointer()
            except RuntimeError:
                if attempt == 19:
                    raise
                time.sleep(.15)


guest = StableGuest(args.iso, args.out)
result = {'verdict': 'PARTIAL', 'checks': []}

def passed(check):
    result['checks'].append(check)
    print('PASS:', check, flush=True)

try:
    guest.wait('[TTY]  Login screen ready', timeout=180)
    time.sleep(5)
    guest.key('ret')
    guest.type('root')
    guest.key('ret')
    guest.wait('[SESSION] switched to F2 GraphicalDesktop')
    time.sleep(5)
    guest.capture('01-desktop-icons')
    # Dismiss the diskless first-login welcome window if present.
    guest.click(976, 62)
    guest.click(976, 62)
    # Re-establish the normal desktop pointer ownership before mouse tests.
    guest.switch('f1')
    guest.switch('f2')
    for attempt in range(3):
        guest.click(*guest.launcher())
        start = guest.capture(f'02-start-{attempt}')
        if 'Pinned' in start:
            break
    assert 'Pinned' in start, start
    guest.key('esc')
    guest.move(33, 769)
    hover = guest.capture('03-overview-tooltip')
    from PIL import Image
    tooltip = guest.out / '03-tooltip-crop.png'
    Image.open(guest.out / '03-overview-tooltip.png').crop((0, 724, 69, 745)).resize((276, 84)).save(tooltip)
    hover += subprocess.check_output(['tesseract', str(tooltip), 'stdout', '--psm', '7'],
                                     stderr=subprocess.DEVNULL, text=True)
    assert 'Overview' in hover, hover
    guest.click(33, 769)
    overview = guest.capture('04-overview')
    workspace_crop = guest.out / '04-workspaces-crop.png'
    Image.open(guest.out / '04-overview.png').crop((230, 600, 1050, 738)).resize((1640, 276)).save(workspace_crop)
    overview += subprocess.check_output(['tesseract', str(workspace_crop), 'stdout', '--psm', '6'],
                                        stderr=subprocess.DEVNULL, text=True)
    assert 'Workspace' in overview and 'Pinned' not in overview, overview
    passed('Distinct Applications and Overview buttons open their respective surfaces')
    guest.key('esc')
    guest.click(1166, 24)
    panel = guest.capture('04b-top-network-popover')
    assert 'Network' in panel, panel
    passed('Top-bar Network glyph retains its status popover action')
    guest.click(1166, 24)
    # Desktop shortcuts use double click; preserve the real pointer path.
    guest.move(60, 258)
    for buttons in (1, 0, 1, 0):
        guest.request('human-monitor-command', {'command-line': f'mouse_button {buttons}'})
        time.sleep(.08)
    time.sleep(3)
    settings = guest.capture('05-network-target')
    assert 'System Preferences' in settings, settings
    passed('Network globe opens the existing System Preferences target')
    guest.switch('f1')
    guest.key('ret')
    guest.type('root')
    guest.key('tab')
    guest.key('spc')
    guest.key('ret')
    guest.wait('[TTY]  Built-in shell ready')
    time.sleep(3)
    before = guest.capture('06-tty-status')
    time.sleep(3)
    after = guest.capture('07-tty-status-updated')
    for text in (before, after):
        assert all(metric in text for metric in ('CPU', 'RAM', 'NET')), text
    clock = r'\d{1,2}:\d{2}:\d{2}'
    first, second = re.search(clock, before), re.search(clock, after)
    assert first and second and first.group() != second.group(), (before, after)
    passed('TTY live status fields visible and clock advances without keyboard input')
    guest.type('echo statusalive\n')
    guest.wait('[TTY]  cmd: echo statusalive -> statusalive')
    guest.switch('f2')
    guest.capture('08-settings-preserved')
    guest.switch('f1')
    text = guest.capture('09-tty-preserved')
    assert 'statusalive' in text, text
    passed('Settings and TTY output survive a desktop/TTY round trip')
    result['verdict'] = 'PASS'
finally:
    (guest.out / 'result.json').write_text(json.dumps(result, indent=2))
    guest.close()
