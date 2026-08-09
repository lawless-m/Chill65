import re,sys,os,glob

ROOT="/home/matt/Git/Chill65/crystal-castles"

def load(p):
    return open(p,'rb').read().replace(b'\x00',b'').decode('latin-1').replace('\r','').split('\n')

# ---- pass 1: classify labels ----
ram=set(); rom=set(); equ={}
labdef=re.compile(r'^([A-Za-z_$][A-Za-z0-9_.$]*):{1,2}')
equdef=re.compile(r'^[\t ]*([A-Za-z_$][A-Za-z0-9_.$]*)[\t ]*=[\t ]*([^;]+)')
blk=re.compile(r'^[\t ]*\.BLK[BW]',re.I)

files=[f for f in glob.glob(ROOT+"/*.MAC")]
lines={}
for f in files:
    lines[f]=load(f)

for f in files:
    L=lines[f]
    for i,l in enumerate(L):
        m=labdef.match(l)
        if m:
            name=m.group(1)
            rest=l[m.end():]
            nxt=L[i+1] if i+1<len(L) else ''
            if blk.match(rest) or blk.match(nxt) or re.match(r'^[\t ]*\.BLK',rest,re.I):
                ram.add(name)
            else:
                rom.add(name)
        m2=equdef.match(l)
        if m2 and not l.lstrip().startswith(';'):
            equ[m2.group(1)]=m2.group(2).strip()

rom -= ram

# ---- pass 2: find writes ----
WRITE=re.compile(r'^[\t ]*(STA|STX|STY|INC|DEC|ASL|LSR|ROL|ROR)[\t ]+([^;]+)',re.I)
sym=re.compile(r'[A-Za-z_$][A-Za-z0-9_.$]*')
hits=[]
for f in files:
    for i,l in enumerate(lines[f],1):
        code=l.split(';')[0]
        m=WRITE.match(code)
        if not m: continue
        op=m.group(2).strip()
        if op.startswith('#') or op.upper().startswith('I,'): continue
        for s in sym.findall(op):
            su=s.upper()
            if su in ('X','Y','NY','A','I','ZX','ZY'): continue
            if s in rom:
                hits.append((os.path.basename(f),i,m.group(1),op,s))
                break

print("labels: ROM-image=%d  RAM(.BLK)=%d  equates=%d"%(len(rom),len(ram),len(equ)))
print("writes targeting a ROM-image label: %d\n"%len(hits))
for h in hits:
    print("  %-12s:%-5d %-4s %-28s -> %s"%h)
