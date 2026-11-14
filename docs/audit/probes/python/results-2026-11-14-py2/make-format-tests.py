import ctypes,random,pathlib,json
E=pathlib.Path(__file__).resolve().parent;lib=ctypes.CDLL(None);sn=lib.snprintf;sn.restype=ctypes.c_int
r=random.Random(31300);bits=[0,1,0x8000000000000000,0x7ff0000000000000,0xfff0000000000000,0x7ff8000000000000,0xfff8000000000000,0x7fefffffffffffff,0x0010000000000000]+[r.getrandbits(64) for _ in range(80)]
formats=['%f','%e','%g','%.0f','%.0e','%.0g','%.17g','%#.0f','%#.6g','%+024.9e','%-24.7G','% 018.3f','%.1101f','%.1101e','%.1101g','%#.1101g','%F','%E','%G','%lf']
rows=[]
for b in bits:
 v=ctypes.c_double.from_buffer_copy(b.to_bytes(8,'little'))
 for fmt in formats:
  buf=ctypes.create_string_buffer(4096);n=sn(buf,4096,fmt.encode(),v);assert 0<=n<4096
  rows.append('{0x%016xULL,%s,%s,%d}'%(b,json.dumps(fmt),json.dumps(buf.value.decode()),n))
(E/'format-oracle.h').write_text('struct float_case { unsigned long long bits; const char *fmt,*expected; int length; };\nstatic const struct float_case floats[]={\n'+',\n'.join(rows)+'\n};\n')
(E/'oracle-provenance.json').write_text(json.dumps({'oracle':'host libc snprintf through ctypes.CDLL(None)','host_libc':__import__('platform').libc_ver(),'seed':31300,'binary64_patterns':len(bits),'formats':formats,'cases':len(rows)},indent=2))
