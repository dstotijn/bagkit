#!/usr/bin/env python3
"""Generate deterministic BAG-shaped XML and measure complete release imports."""
import argparse
import json
import platform
import resource
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

HEADER = '<sl-bag-extract:bagStand xmlns:sl-bag-extract="urn:bag" xmlns:sl="urn:stand" xmlns:Objecten="urn:objects" xmlns:Objecten-ref="urn:refs" xmlns:Historie="urn:history" xmlns:gml="http://www.opengis.net/gml/3.2"><sl:standBestand>\n'
FOOTER = '</sl:standBestand></sl-bag-extract:bagStand>\n'


def field(k, value):
    return f'<Objecten:{k}>{value}</Objecten:{k}>'


def ref(k, kind, identifier):
    return field(k, f'<Objecten-ref:{kind}Ref>{identifier}</Objecten-ref:{kind}Ref>')


def obj(kind, identifier, body):
    return f'<sl:stand><sl-bag-extract:bagObject><Objecten:{kind}>'+field('identificatie',identifier)+body+'<Objecten:voorkomen><Historie:Voorkomen><Historie:beginGeldigheid>2000-01-01</Historie:beginGeldigheid></Historie:Voorkomen></Objecten:voorkomen>'+f'</Objecten:{kind}></sl-bag-extract:bagObject></sl:stand>\n'


def generate(path, count, columns=1000, spacing=1, building_links=1):
    with path.open('w') as f:
        f.write(HEADER)
        f.write(obj('Woonplaats',1234,field('naam','Teststad')+field('status','Woonplaats aangewezen')+field('geconstateerd','N')))
        for i in range((count+99)//100):
            f.write(obj('OpenbareRuimte',f'{i:016d}',field('naam',f'Straat {i}')+field('status','Naamgeving uitgegeven')+field('type','Weg')+ref('ligtIn','Woonplaats',1234)))
        for i in range(count):
            f.write(obj('Nummeraanduiding',f'{i:016d}',field('huisnummer',i%100+1)+field('postcode','1234AB')+field('status','Naamgeving uitgegeven')+ref('ligtAan','OpenbareRuimte',f'{i//100:016d}')))
        for i in range(count):
            x, y = 155000+(i%columns)*spacing, 463000+(i//columns)*spacing
            point=f'<Objecten:geometrie><Objecten:punt><gml:Point><gml:pos>{x} {y} 0</gml:pos></gml:Point></Objecten:punt></Objecten:geometrie>'
            f.write(obj('Verblijfsobject',f'{i:016d}',field('status','Verblijfsobject in gebruik')+field('oppervlakte',100)+field('gebruiksdoel','woonfunctie')+point+ref('heeftAlsHoofdadres','Nummeraanduiding',f'{i:016d}')+''.join(ref('maaktDeelUitVan','Pand',f'{(i+j)%count:016d}') for j in range(min(building_links,count)))))
        for i in range(count):
            polygon='<Objecten:geometrie><gml:Polygon srsDimension="2"><gml:exterior><gml:LinearRing><gml:posList>155000 463000 155010 463000 155010 463010 155000 463000</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></Objecten:geometrie>'
            f.write(obj('Pand',f'{i:016d}',field('status','Pand in gebruik')+field('oorspronkelijkBouwjaar',2000)+polygon))
        f.write(FOOTER)


def worker(args):
    dest = Path(args.work).resolve()
    dest.mkdir(parents=True,exist_ok=True)
    source = str(Path(args.input).resolve())
    started = time.perf_counter()
    command = [str(Path(args.bagkit).resolve()), 'convert', '--force', '--output', str(dest/'bag.sqlite'), '--cache-mib', str(args.cache_mib), source]
    if args.no_geometry: command += ['--exclude', 'geometrie']
    subprocess.run(command, check=True, stdout=subprocess.DEVNULL)
    elapsed = time.perf_counter()-started
    unit = 1024*1024 if sys.platform == 'darwin' else 1024
    peak = max(resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss,resource.getrusage(resource.RUSAGE_SELF).ru_maxrss)/unit
    print(json.dumps({'seconds':round(elapsed,3),'peak_rss_mib':round(peak,1),'database_mib':round((dest/'bag.sqlite').stat().st_size/1024**2,1)}))


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--count',type=int,default=100000)
    p.add_argument('--columns', type=int, default=1000, help='dwellings per generated grid row')
    p.add_argument('--spacing', type=int, default=1, help='grid spacing in RD meters')
    p.add_argument('--building-links', type=int, default=1, help='distinct linked buildings per dwelling')
    p.add_argument('--cache-mib',type=int,default=64)
    p.add_argument('--no-geometry', action='store_true', help='omit building and addressable object footprints')
    p.add_argument('--repeats',type=int,default=1)
    p.add_argument('--directory',type=Path,default=Path('tmp/benchmark'))
    p.add_argument('--bagkit',default='target/release/bagkit')
    p.add_argument('--worker',action='store_true',help=argparse.SUPPRESS)
    p.add_argument('--input',help=argparse.SUPPRESS)
    p.add_argument('--work',help=argparse.SUPPRESS)
    args=p.parse_args()
    if args.worker:
        worker(args); return
    if min(args.count, args.repeats, args.cache_mib, args.columns, args.spacing, args.building_links) < 1: p.error('count, repeats, cache-mib, columns, spacing and building-links must be positive')
    args.directory.mkdir(parents=True,exist_ok=True)
    source=args.directory/'generated.xml'
    generate(source,args.count,args.columns,args.spacing,args.building_links)
    hardware=platform.processor() or platform.machine()
    if sys.platform=='darwin': hardware=subprocess.check_output(['sysctl','-n','machdep.cpu.brand_string'],text=True).strip()
    print(json.dumps({'hardware':hardware,'os':platform.platform(),'python':platform.python_version(),'dwellings':args.count,'columns':args.columns,'spacing_m':args.spacing,'building_links':min(args.building_links,args.count),'xml_mib':round(source.stat().st_size/1024**2,1),'note':'synthetic dataset; full import including indexes, spatial index and footprints unless omitted; peak RSS is maximum of process and children, not their sum'}),flush=True)
    for _ in range(args.repeats):
        cmd=[sys.executable,str(Path(__file__).resolve()),'--worker','--input',str(source),'--work',str(args.directory/'run'),'--bagkit',args.bagkit,'--cache-mib',str(args.cache_mib)]
        if args.no_geometry: cmd+=['--no-geometry']
        subprocess.run(cmd,check=True)

if __name__=='__main__': main()
