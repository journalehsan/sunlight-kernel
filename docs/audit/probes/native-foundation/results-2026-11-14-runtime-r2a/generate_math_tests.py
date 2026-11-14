import math,random,struct,ctypes,pathlib,json
p=pathlib.Path(__file__).resolve().parent; rng=random.Random(20261114)
def bits(x):return struct.unpack('Q',struct.pack('d',x))[0]
def f32(x):return ctypes.c_float(x).value
lib=ctypes.CDLL('libm.so.6',use_errno=True)
for name in ['atanhf','lgammaf']:
 f=getattr(lib,name);f.argtypes=[ctypes.c_float];f.restype=ctypes.c_float
refgamma=lib.lgamma_r; refgamma.argtypes=[ctypes.c_double,ctypes.POINTER(ctypes.c_int)];refgamma.restype=ctypes.c_double
rows=[]
def add(kind,x,y,z=0,ex=0):rows.append((kind,bits(x),bits(y),bits(z),ex))
common=[0.,-0.,float('inf'),-float('inf'),float('nan'),5e-324,-5e-324,2**-1022,-2**-1022,1.,-1.,2.,-2.,1e308,-1e308]
xs=common+[math.nextafter(1.,0.),math.nextafter(-1.,0.),math.nextafter(1.,2.),2**-33,-2**-33]+[rng.uniform(-.99999999,.99999999) for _ in range(1800)]
for x in xs:
 try:y=math.atanh(x)
 except ValueError:y=math.copysign(math.inf,x) if abs(x)==1 else math.nan
 add(0,x,y)
 xf=f32(x)
 add(4,xf,float(lib.atanhf(xf)))
xs=common+[.5,-.5,-1.5,-2.5,171.,1e305]+[rng.uniform(-60,180) for _ in range(2000)]
for x in xs:
 sign=ctypes.c_int();y=refgamma(x,ctypes.byref(sign));add(1,x,y,ex=sign.value)
 xf=f32(x);add(5,xf,float(lib.lgammaf(xf)))
xs=common+[math.ldexp(rng.uniform(-1,1),rng.randrange(-1073,1024)) for _ in range(2400)]
for x in xs:
 f,ex=math.frexp(x);add(2,x,f,ex=ex)
 f,i=math.modf(x);add(3,x,f,i)
 xf=f32(x);f,ex=math.frexp(xf);add(6,xf,f,ex=ex)
 f,i=math.modf(xf);add(7,xf,f,i)
text='struct vector { int kind; unsigned long x,y,z; int exponent; };\nstatic const struct vector vectors[] = {\n'
text+=''.join('{%d,0x%xUL,0x%xUL,0x%xUL,%d},\n'%row for row in rows)+'};\n';(p/'math-vectors.h').write_text(text)
(p/'numerical-reference.json').write_text(json.dumps(dict(reference='host Python math and glibc libm.so.6; independent of vendored Rust libm',seed=20261114,vectors=len(rows),per_kind={str(k):sum(r[0]==k for r in rows) for k in range(8)},tolerances={'atanh':'4 ULP, tiny/subnormal/zero exact','lgamma':'8e-14 * max(1,abs(reference))','atanhf':'4 float ULP','lgammaf':'4e-6 * max(1,abs(reference))','frexp/modf and float variants':'bit exact including signed zero; NaN classification'},note='bounded validation, not full libm conformance'),indent=2))
