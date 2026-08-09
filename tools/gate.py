import re,glob,os,collections
ROOT="/home/matt/Git/Chill65/crystal-castles"
def load(p): return open(p,'rb').read().replace(b'\x00',b'').decode('latin-1').replace('\r','').split('\n')

RAMFILES={'CG.MAC'}                      # pure storage declarations
labdef=re.compile(r'^([A-Za-z_$0-9][A-Za-z0-9_.$]*):{1,2}')
XFER=re.compile(r'^[\t ]*(JMP|JSR|BNE|BEQ|BCC|BCS|BMI|BPL|BVC|BVS)[\t ]+([^;]+)',re.I)
sym=re.compile(r'[A-Za-z_$][A-Za-z0-9_.$]*')

for tree in ['', 'version-2', 'version-3']:
    d=os.path.join(ROOT,tree)
    files=sorted(glob.glob(d+"/*.MAC"))
    if not files: continue
    ramsyms=set(); romlab=set(); lines={}
    for f in files:
        lines[f]=load(f)
        base=os.path.basename(f)
        for l in lines[f]:
            m=labdef.match(l)
            if m: (ramsyms if base in RAMFILES else romlab).add(m.group(1))
    # CRP storage regions -> RAM (labels after .=0F0 / .=0B00 until next .ASECT)
    bad=[]; local=0; named=0; computed=[]
    for f in files:
        base=os.path.basename(f)
        if base in RAMFILES: continue
        for i,l in enumerate(lines[f],1):
            code=l.split(';')[0]
            m=XFER.match(code)
            if not m: continue
            t=m.group(2).strip()
            if t=='.' or re.match(r'^\.[+-]',t): continue          # PC-relative
            if re.match(r'^\d+\$$',t): local+=1; continue           # local label
            s=sym.findall(t)
            if not s:
                computed.append((base,i,m.group(1),t)); continue
            named+=1
            for x in s:
                if x in ramsyms: bad.append((base,i,m.group(1),t))
    print("### tree: %-10s  files=%d" % (tree or '(root)', len(files)))
    print("    control transfers: %d to named labels, %d to local (n$) labels" % (named,local))
    print("    targets resolving into RAM-declared symbols : %d" % len(bad))
    print("    targets that are pure expressions (computed): %d" % len(computed))
    for c in computed[:10]: print("        ",c)
    for b in bad[:10]: print("      RAM!",b)
    print()
