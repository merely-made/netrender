"""Read-only simple-glyf coordinate/flatten decision audit, no rasterization.
Models local primary-source FreeType/F26Dot6 arithmetic; not a Skrifa execution.
Writes safe aggregate metrics and bounded segment samples, not font/scene blobs.
"""
import struct,json,math,hashlib
from pathlib import Path
OUT=Path(__file__).parent
ROOT=OUT.parent
FONT=Path('/System/Library/Fonts/Helvetica.ttc')
blob=FONT.read_bytes()
receipt=json.loads((ROOT/'glyph-all/receipt.json').read_text())
case=next(c for c in receipt['cases'] if c['name']=='glyph-original-run')
info=case['details']; assert info['font_index']==0 and info['font_size']==14 and info['font_upem']==2048
assert hashlib.sha256(blob).hexdigest()==info['font_sha256'][info['font_id']]
def u16(p):return struct.unpack_from('>H',blob,p)[0]
def s16(p):return struct.unpack_from('>h',blob,p)[0]
def u32(p):return struct.unpack_from('>I',blob,p)[0]
face=u32(12) if blob[:4]==b'ttcf' else 0
tables={blob[p:p+4].decode('ascii'):(u32(p+8),u32(p+12)) for p in range(face+12,face+12+16*u16(face+4),16)}
head=tables['head'][0];assert u16(head+18)==2048
fmt=s16(head+50);loca=tables['loca'][0];glyf=tables['glyf'][0]
def points(gid):
 start=(u32(loca+4*gid) if fmt else u16(loca+2*gid)*2)
 end=(u32(loca+4*(gid+1)) if fmt else u16(loca+2*(gid+1))*2)
 if start==end:return []
 pos=glyf+start;n=s16(pos)
 if n<0:raise ValueError(f'composite glyph {gid} is not in this bounded simple-glyf audit')
 if n==0:return []
 ends=[u16(pos+10+2*i) for i in range(n)]; count=ends[-1]+1
 pos+=10+2*n;pos+=2+u16(pos)
 flags=[]
 while len(flags)<count:
  flag=blob[pos];pos+=1;flags.append(flag)
  if flag&8:
   repeat=blob[pos];pos+=1;flags.extend([flag]*repeat)
 coords=[]
 for short,same in [(2,16),(4,32)]:
  values=[];current=0
  for flag in flags:
   if flag&short:
    delta=blob[pos];pos+=1
    if not flag&same:delta=-delta
   elif flag&same:delta=0
   else:delta=s16(pos);pos+=2
   current+=delta;values.append(current)
  coords.append(values)
 result=[];previous=0
 for end in ends:
  result.append([(coords[0][i],coords[1][i],bool(flags[i]&1)) for i in range(previous,end+1)]);previous=end+1
 return result
def round_fixed(p,scale):
 value=p*scale
 return (value+32768-(1 if value<0 else 0))>>16
def midpoint(a,b):return tuple(int((x+y)/2) for x,y in zip(a,b))
def path(contours,scale,ratio):
 ops=[]
 for contour in contours:
  pts=[((round_fixed(x,scale),round_fixed(y,scale)),on) for x,y,on in contour]
  first,last=pts[0],pts[-1]
  if first[1]:start=first[0];remaining=pts[1:]
  elif last[1]:start=last[0];remaining=pts[:-1]
  else:start=midpoint(first[0],last[0]);remaining=pts
  convert=lambda p:tuple(v/64*ratio for v in p)
  ops.append(('M',convert(start)));pending=None
  for p,on in remaining+[(start,True)]:
   if pending is None:
    if on:ops.append(('L',convert(p)))
    else:pending=p
   elif on:ops.append(('Q',convert(pending),convert(p)));pending=None
   else:ops.append(('Q',convert(pending),convert(midpoint(pending,p))));pending=p
  ops.append(('Z',))
 return ops
def dist(p,a,b):
 d=(b[0]-a[0],b[1]-a[1]);l=d[0]**2+d[1]**2
 t=max(0,min(1,((p[0]-a[0])*d[0]+(p[1]-a[1])*d[1])/l)) if l else 0
 return math.hypot(p[0]-a[0]-t*d[0],p[1]-a[1]-t*d[1])
def approxint(x):return x/(1-.67+math.sqrt(math.sqrt(.67**4+.25*x*x)))
def subdivision(a,b,c):
 deviation=dist(b,a,c)
 if deviation<=.5:return {'line_simplified':True,'n':1,'control_to_chord':deviation}
 d01=(b[0]-a[0],b[1]-a[1]);d12=(c[0]-b[0],c[1]-b[1]);dd=(d01[0]-d12[0],d01[1]-d12[1]);cross=(c[0]-a[0])*dd[1]-(c[1]-a[1])*dd[0]
 if cross==0:return {'line_simplified':False,'n':1,'control_to_chord':deviation,'degenerate':True}
 x0=(d01[0]*dd[0]+d01[1]*dd[1])/cross;x2=(d12[0]*dd[0]+d12[1]*dd[1])/cross
 scale=abs(cross/(math.hypot(*dd)*(x2-x0)));da=abs(approxint(x2)-approxint(x0));s=math.sqrt(scale)
 if (x0>0)==(x2>0):value=da*s
 else:value=.5*da/approxint(.5/s)
 return {'line_simplified':False,'n':max(1,math.ceil(value)),'control_to_chord':deviation,'estimated_subdivision_value':value}
entries=[];crossings=[];max_coordinate_delta=0.;pointcount=0
for gid in sorted({g['id'] for g in info['glyphs']}):
 contours=points(gid);draw=path(contours,28672,1.);upem=path(contours,4194304,14/2048)
 assert len(draw)==len(upem) and all(a[0]==b[0] for a,b in zip(draw,upem))
 delta=max((abs(a-b) for x,y in zip(draw,upem) for p,q in zip(x[1:],y[1:]) for a,b in zip(p,q)),default=0)
 max_coordinate_delta=max(max_coordinate_delta,delta);pointcount+=sum(len(o)-1 for o in draw)
 a0=b0=None;segment=0
 for d,u in zip(draw,upem):
  if d[0]=='M':a0=d[1];b0=u[1]
  elif d[0]=='L':a0=d[1];b0=u[1]
  elif d[0]=='Q':
   sd=subdivision(a0,d[1],d[2]);su=subdivision(b0,u[1],u[2])
   if sd['line_simplified']!=su['line_simplified'] or sd['n']!=su['n']:
    crossings.append({'glyph_id':gid,'segment':segment,'draw_size_points':[a0,d[1],d[2]],'upem_scaled_points':[b0,u[1],u[2]],'draw_size_flatten_decision':sd,'upem_scaled_flatten_decision':su,'run_occurrences':[{'x':g['x'],'y':g['y']} for g in info['glyphs'] if g['id']==gid]})
   a0=d[2];b0=u[2];segment+=1
 entries.append({'glyph_id':gid,'contours':len(contours),'ops':len(draw),'max_coordinate_delta':delta})
result={'schema_version':1,'scope':'Offline primary-source arithmetic model, no Skrifa or renderer execution. Simple glyf only; exact validated font/face/run; no variation coordinates.','font_path':str(FONT),'font_sha256':hashlib.sha256(blob).hexdigest(),'receipt_sha256':hashlib.sha256((ROOT/'glyph-all/receipt.json').read_bytes()).hexdigest(),'font_size':14,'font_upem':2048,'ratio':14/2048,'draw_size_fixed_scale_bits':28672,'upem_fixed_scale_bits':4194304,'max_outline_coordinate_delta_pixels':max_coordinate_delta,'emitted_controlpoint_count_unique_glyphs':pointcount,'glyphs':entries,'sparse_subdivision_decision_crossings':crossings,'source_contract':{'fixed_multiply':'(a*b + 32768 - (a*b<0)) >> 16','point_midpoint':'signed integer division (a+b)/2 truncates toward zero','sparse_quad_simplification':'control-to-line-segment distance <=0.5 => line, equivalent curve deviation bound <=0.25','sparse_quad_subdivision':'ceil(approx integral variation at SQRT_TOL=0.5), minimum1'}}
(OUT/'glyph-coordinate-audit.json').write_text(json.dumps(result,indent=2)+'\n')
print('glyphs',len(entries),'max coordinate delta',max_coordinate_delta,'decision crossings',len(crossings)); print(json.dumps(crossings,indent=2))
