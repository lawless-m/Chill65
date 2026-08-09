# Region-aware, macro-aware SMC scan. Supersedes smc.py (kept for the record).
import re,glob,os
ROOT="/home/matt/Git/Chill65/crystal-castles"
ROMBASE=0xA000

def load(p): return open(p,'rb').read().replace(b'\x00',b'').decode('latin-1').replace('\r','').split('\n')

SYM=re.compile(r'[A-Za-z_$.][A-Za-z0-9_.$]*')
LAB=re.compile(r'^([A-Za-z_$0-9][A-Za-z0-9_.$]*):{1,2}')
EQU=re.compile(r'^[\t ]*([A-Za-z_$.][A-Za-z0-9_.$]*)[\t ]*={1,2}[\t ]*([^;]+)')
OPSYM=re.compile(r'(?<![0-9A-Za-z_.$])[A-Za-z_$][A-Za-z0-9_.$]*')
STORE=re.compile(r'^[\t ]*(STA|STX|STY|INC|DEC|ASL|LSR|ROL|ROR)[\t ]+([^;]+)',re.I)
XFER=re.compile(r'^[\t ]*(JMP|JSR|BNE|BEQ|BCC|BCS|BMI|BPL|BVC|BVS)[\t ]+([^;]+)',re.I)
MACD=re.compile(r'^[\t ]*\.MACRO[\t ]+(\.?[A-Za-z_$][A-Za-z0-9_.$]*)[\t ]*(.*)',re.I)

def evalx(expr,equ,depth=0):
    if depth>8: return None
    e=expr.split(';')[0].strip()
    if not e or '.' == e or re.search(r'(^|[^A-Za-z0-9_.$])\.($|[^A-Za-z0-9_.$])',e): return None  # refs '.'
    def sub(m):
        s=m.group(0)
        if re.fullmatch(r'\d+\.',s): return s[:-1]          # MACRO-11 decimal
        if re.fullmatch(r'[0-9][0-9A-Fa-f]*',s): return str(int(s,16))
        v=evalx(equ.get(s,''),equ,depth+1) if s in equ else None
        return str(v) if v is not None else 'X'
    e2=re.sub(r'[A-Za-z0-9_.$]+',sub,e)
    if 'X' in e2: return None
    try: return eval(e2,{'__builtins__':{}})
    except Exception: return None

for tree in ['','version-2','version-3']:
    files=sorted(glob.glob(os.path.join(ROOT,tree,'*.MAC')))
    if not files: continue
    lines={f:load(f) for f in files}
    # equates (outside macro bodies)
    equ={}
    for f in files:
        inm=0
        for l in lines[f]:
            if MACD.match(l): inm+=1; continue
            if re.match(r'^[\t ]*\.ENDM',l,re.I): inm=max(0,inm-1); continue
            if inm: continue
            m=EQU.match(l)
            if m and not l.lstrip().startswith(';') and m.group(1) != '.' and m.group(2).strip() != '.': equ[m.group(1)]=m.group(2)
    # macros: formals + which formals are stored to / jumped to (2 passes for nesting)
    macs={}
    for f in files:
        cur=None
        for l in lines[f]:
            m=MACD.match(l)
            if m:
                cur=m.group(1).upper(); macs[cur]=([a.lstrip('?') for a in re.split(r'[,\t ]+',m.group(2).strip()) if a],[])
                continue
            if re.match(r'^[\t ]*\.ENDM',l,re.I): cur=None; continue
            if cur: macs[cur][1].append(l.split(';')[0])
    stf={k:set() for k in macs}; jtf={k:set() for k in macs}
    for _ in range(2):
        for k,(formals,body) in macs.items():
            for b in body:
                ms=STORE.match(b); mx=XFER.match(b)
                if ms:
                    for s in SYM.findall(ms.group(2)):
                        if s in formals: stf[k].add(formals.index(s))
                if mx:
                    for s in SYM.findall(mx.group(2)):
                        if s in formals: jtf[k].add(formals.index(s))
                t=b.strip().split('\t')[0].split(' ')[0].upper()
                if t in macs and t!=k:  # nested macro call
                    args=[a for a in re.split(r'[,\t ]+',b.strip()[len(t):].strip()) if a]
                    for i,a in enumerate(args):
                        for s in SYM.findall(a):
                            if s in formals and i in stf.get(t,set()): stf[k].add(formals.index(s))
                            if s in formals and i in jtf.get(t,set()): jtf[k].add(formals.index(s))
    def rept0_mask(ls):
        # True at lines inside a .REPT block whose count evaluates to 0
        mask=[False]*len(ls); depth=0; zero_at=None
        for i,l in enumerate(ls):
            m=re.match(r'^[\t ]*\.REPT[\t ]+([^;]+)',l,re.I)
            if m:
                depth+=1
                if zero_at is None and evalx(m.group(1),equ)==0: zero_at=depth
            if zero_at is not None: mask[i]=True
            if re.match(r'^[\t ]*\.ENDR',l,re.I):
                if zero_at==depth: zero_at=None
                depth=max(0,depth-1)
        return mask
    r0={f:rept0_mask(lines[f]) for f in files}
    # label regions: walk the real assembly stream from each build root,
    # following .INCLUDE, so region context crosses file boundaries.
    byname={os.path.basename(f):f for f in files}
    def stream(name,depth=0):
        f=byname.get(name) or byname.get(name+'.MAC')
        if not f or depth>8: return
        inm=0
        for li,l in enumerate(lines[f]):
            if r0[f][li]: continue
            if MACD.match(l): inm+=1; continue
            if re.match(r'^[\t ]*\.ENDM',l,re.I): inm=max(0,inm-1); continue
            if inm: continue
            mi=re.match(r'^[\t ]*\.INCLUDE[\t ]+([A-Za-z0-9_.$]+)',l,re.I)
            if mi and not l.lstrip().startswith(';'):
                yield from stream(mi.group(1).upper(),depth+1); continue
            yield l
    reg={}
    for root in ['CRF.MAC','C99.MAC','CRP.MAC']:
        cur='ROM'
        for l in stream(root):
            me=re.match(r'^[\t ]*\.[\t ]*=[\t ]*([^;]+)',l) or re.match(r'^[\t ]*\.=([^;]+)',l)
            if me:
                v=evalx(me.group(1),equ)
                if v is not None: cur='RAM' if v<ROMBASE else 'ROM'
                continue
            m=LAB.match(l)
            if m: reg[m.group(1)]=cur
    def cls(sym,depth=0):
        if sym in reg: return reg[sym]
        if sym in equ:
            v=evalx(equ[sym],equ)
            if v is not None: return 'RAM' if v<ROMBASE else 'ROMEQU'
            if depth<4:   # alias chain: SYM = LABEL or LABEL+const -> region of the label
                syms=[s for s in OPSYM.findall(equ[sym]) if s != '.']
                regions={cls(s,depth+1) for s in syms if not s.replace('.','').isdigit()}
                regions.discard('RAM') if False else None
                known=regions-{'UNK'}
                if known and 'ROM' not in known and 'ROMEQU' not in known: return 'RAM'
                if 'ROM' in known or 'ROMEQU' in known: return 'ROM'
        return 'UNK'
    def check(op_syms):
        worst=None
        for s in op_syms:
            c=cls(s)
            if c in ('ROM','ROMEQU'): return ('HIT',s,c)
            if c=='UNK': worst=('UNK',s,c)
        return worst or ('OK',None,None)
    bare=0;mac=0;hits=[];unk=[];jhits=[]
    for f in files:
        base=os.path.basename(f); inm=0
        for i,l in enumerate(lines[f],1):
            if MACD.match(l): inm+=1; continue
            if re.match(r'^[\t ]*\.ENDM',l,re.I): inm=max(0,inm-1); continue
            if inm: continue
            code=l.split(';')[0]
            ms=STORE.match(code)
            if ms:
                op=ms.group(2).strip()
                if op.startswith('#') or op.upper().startswith('I,'): continue
                bare+=1
                r=check([s for s in OPSYM.findall(op) if s.upper() not in ('X','Y','NY','A','I','Z','ZX','ZY','NX','AY','AX')])
                if r[0]=='HIT': hits.append((base,i,'bare',code.strip()))
                elif r[0]=='UNK': unk.append((base,i,'bare',code.strip(),r[1]))
                continue
            t=code.strip().split('\t')[0].split(' ')[0].upper()
            if t in macs and (stf[t] or jtf[t]):
                args=[a for a in re.split(r'[,\t ]+',code.strip()[len(t):].strip()) if a]
                for idx in stf[t]:
                    if idx<len(args):
                        mac+=1
                        r=check([s for s in OPSYM.findall(args[idx]) if s.upper() not in ('X','Y','NY','A','I','Z','ZX','ZY','NX','AY','AX')])
                        if r[0]=='HIT': hits.append((base,i,t,code.strip()))
                        elif r[0]=='UNK': unk.append((base,i,t,code.strip(),r[1]))
                for idx in jtf[t]:
                    if idx<len(args):
                        tgt=args[idx]
                        if re.fullmatch(r'\d+\$',tgt): continue
                        for s in OPSYM.findall(tgt):
                            if cls(s)=='RAM': jhits.append((base,i,t,code.strip()))
    print("### %-10s files=%d  bare-stores=%d  macro-stores=%d" % (tree or '(root)',len(files),bare,mac))
    print("    stores hitting ROM-region symbols : %d" % len(hits))
    for h in hits: print("      HIT ",h)
    print("    macro-arg transfers to RAM labels : %d" % len(jhits))
    for h in jhits: print("      JHIT",h)
    print("    unresolved (triage)               : %d" % len(unk))
    for u in unk[:15]: print("      UNK ",u)
    print()
