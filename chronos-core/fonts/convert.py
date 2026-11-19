from pathlib import Path
import hashlib
import argparse
parser=argparse.ArgumentParser()
parser.add_argument("source", type=Path, help="directory containing upstream unscii-8.hex and unscii-16.hex")
args=parser.parse_args()
p=Path(__file__).resolve().parent
controls='\0☺☻♥♦♣♠•◘○◙♂♀♪♫☼►◄↕‼¶§▬↨↑↓→←∟↔▲▼'
# Original SunlightOS drawings, CC0; missing Unicode symbols, not fallbacks.
extra={15:bytes.fromhex('24995a3c3c5a9924'),127:bytes.fromhex('00183c6666427e00'),169:bytes.fromhex('00007e0606060000'),249:bytes.fromhex('0000001818000000')}
for h in (8,16):
 gs={int(k,16):bytes.fromhex(v) for k,v in (l.split(':') for l in (args.source / f'unscii-{h}.hex').read_text().splitlines())};out=b''
 for i in range(256):
  ch=controls[i] if i<32 else '⌂' if i==127 else bytes([i]).decode('cp437')
  g=bytes(h) if i==0 else (extra[i] if h==8 else bytes(b for b in extra[i] for _ in range(2))) if i in extra else gs[ord(ch)]
  assert len(g)==h
  out+=g
 (p/f'cp437-{h}.bin').write_bytes(out)
 print(h,len(out),hashlib.sha256(out).hexdigest())
