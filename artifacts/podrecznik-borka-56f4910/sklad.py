#!/usr/bin/env python3
"""Składa podręcznik z Markdown i wektorowych rysunków Graphviz."""
from pathlib import Path
import html
import json
import re
import subprocess
import textwrap
import xml.etree.ElementTree as ET

import markdown
from reportlab.lib import colors
from reportlab.lib.enums import TA_LEFT
from reportlab.lib.styles import ParagraphStyle
from reportlab.lib.units import mm
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.platypus import (
    BaseDocTemplate, Frame, PageTemplate, Paragraph, Spacer, PageBreak,
    KeepTogether, Flowable, Table, TableStyle,
)
from reportlab.platypus.tableofcontents import TableOfContents
from svglib.svglib import svg2rlg
from svglib.fonts import register_font_family
from reportlab.graphics import renderPDF

ROOT = Path(__file__).resolve().parent
SHA = '56f4910a24b689087fe2dee30247651b7b432268'
W, H = 190*mm, 250*mm
M = 21*mm
CW = W-2*M
INK = colors.HexColor('#142e3b')
TEAL = colors.HexColor('#087e83')
COPPER = colors.HexColor('#b15f36')
PALE = colors.HexColor('#edf4f4')

FONTDIR = Path('/usr/share/fonts/truetype/ibm-plex')
for name, file in [
    ('Body','IBMPlexSerif-Regular.ttf'),('BodyB','IBMPlexSerif-SemiBold.ttf'),
    ('BodyI','IBMPlexSerif-Italic.ttf'),('Sans','IBMPlexSans-Regular.ttf'),
    ('SansB','IBMPlexSans-SemiBold.ttf'),('SansI','IBMPlexSans-Italic.ttf'),
    ('Mono','IBMPlexMono-Regular.ttf'),
]:
    pdfmetrics.registerFont(TTFont(name, str(FONTDIR/file)))
pdfmetrics.registerFont(TTFont('DejaVu Sans', '/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf'))
register_font_family('DejaVu Sans',
    '/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf',
    bold='/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf',
    italic='/usr/share/fonts/truetype/dejavu/DejaVuSans-Oblique.ttf',
    bolditalic='/usr/share/fonts/truetype/dejavu/DejaVuSans-BoldOblique.ttf')
pdfmetrics.registerFontFamily('Body', normal='Body',bold='BodyB',italic='BodyI',boldItalic='BodyB')
pdfmetrics.registerFontFamily('Sans', normal='Sans',bold='SansB',italic='SansI',boldItalic='SansB')

styles = {
    'body': ParagraphStyle('body',fontName='Body',fontSize=10.2,leading=15,
        textColor=INK,spaceAfter=7.2,allowWidows=0,allowOrphans=0),
    'h1': ParagraphStyle('h1',fontName='SansB',fontSize=25,leading=29,
        textColor=INK,spaceAfter=18,keepWithNext=True),
    'h2': ParagraphStyle('h2',fontName='SansB',fontSize=14,leading=18,
        textColor=TEAL,spaceBefore=16,spaceAfter=8,keepWithNext=True),
    'h3': ParagraphStyle('h3',fontName='SansB',fontSize=11.3,leading=15,
        textColor=INK,spaceBefore=10,spaceAfter=6,keepWithNext=True),
    'small': ParagraphStyle('small',fontName='Sans',fontSize=8.3,leading=11.5,
        textColor=INK,spaceAfter=7),
    'caption': ParagraphStyle('caption',fontName='Sans',fontSize=8.4,leading=12,
        textColor=INK,spaceBefore=6,spaceAfter=13),
    'cell': ParagraphStyle('cell',fontName='Sans',fontSize=8.2,leading=11,
        textColor=INK),
    'bullet': ParagraphStyle('bullet',fontName='Body',fontSize=10.2,leading=15,
        textColor=INK,leftIndent=13,firstLineIndent=-10,spaceAfter=5),
}

def inline(e):
    """Konwersja bezpiecznego drzewa HTML na podzbiór znaczników ReportLab."""
    out = html.escape(e.text or '')
    for child in e:
        content = inline(child)
        if child.tag in ('em','strong','b','i'):
            tag = {'em':'i','strong':'b'}.get(child.tag, child.tag)
            out += f'<{tag}>{content}</{tag}>'
        elif child.tag == 'code':
            out += f'<font name="Mono" size="8.5" color="#087e83">{content}</font>'
        elif child.tag == 'a':
            href = html.escape(child.attrib.get('href',''),quote=True)
            out += f'<a href="{href}" color="#087e83">{content}</a>'
        elif child.tag == 'br':
            out += '<br/>'
        else:
            out += content
        out += html.escape(child.tail or '')
    return out

class Code(Flowable):
    def __init__(self, source, language='', continued=False):
        super().__init__()
        self.source, self.language, self.continued = source, language, continued
        self.lines = []
    def wrap(self, aw, ah):
        self.width = aw
        self.fs, self.leading = 7.1, 10.2
        maxchars = max(24,int((aw-24)/pdfmetrics.stringWidth('x','Mono',self.fs)))
        self.lines = []
        for line in self.source.expandtabs(4).splitlines() or ['']:
            self.lines.extend(textwrap.wrap(line,maxchars,replace_whitespace=False,
                drop_whitespace=False,break_long_words=True,break_on_hyphens=False) or [''])
        self.height = 30+len(self.lines)*self.leading
        return self.width,self.height
    def split(self,aw,ah):
        self.wrap(aw,ah)
        n = int((ah-30)/self.leading)
        if n<3:
            return []
        return [Code('\n'.join(self.lines[:n]),self.language,self.continued),
                Code('\n'.join(self.lines[n:]),self.language,True)]
    def draw(self):
        c=self.canv
        c.setFillColor(PALE)
        c.roundRect(0,0,self.width,self.height,4,fill=1,stroke=0)
        c.setFillColor(TEAL)
        c.rect(0,0,2,self.height,fill=1,stroke=0)
        c.setFont('SansB',7)
        c.drawString(12,self.height-12,(self.language.upper() or 'FRAGMENT')+
                     (' · ciąg dalszy' if self.continued else ''))
        c.setFillColor(INK)
        c.setFont('Mono',self.fs)
        y=self.height-25
        for line in self.lines:
            c.drawString(12,y,line)
            y-=self.leading

class VectorFigure(Flowable):
    def __init__(self,path):
        super().__init__()
        self.drawing=svg2rlg(str(path))
        assert self.drawing is not None,path
    def wrap(self,aw,ah):
        self.scale=min(aw/self.drawing.width,310/self.drawing.height,1.12)
        self.width=self.drawing.width*self.scale
        self.height=self.drawing.height*self.scale
        self.hAlign='CENTER'
        return self.width,self.height
    def draw(self):
        self.canv.saveState()
        self.canv.scale(self.scale,self.scale)
        renderPDF.draw(self.drawing,self.canv,0,0)
        self.canv.restoreState()

class Cover(Flowable):
    def wrap(self,aw,ah):
        self.width=aw; self.height=ah-1
        return aw,self.height
    def draw(self):
        c=self.canv
        c.setFillColor(INK)
        c.rect(-M,-M,W,H,fill=1,stroke=0)
        c.setFillColor(colors.HexColor('#72d2c2'))
        c.setFont('SansB',10)
        c.drawString(0,self.height-30,'JĘZYK  /  KOMPILATOR  /  PAMIĘĆ')
        c.setFillColor(colors.white)
        c.setFont('SansB',61)
        c.drawString(-3,self.height-115,'BORK')
        c.setFont('Sans',26)
        c.drawString(0,self.height-158,'Budowa kompilatora')
        c.setFont('Body',13)
        c.drawString(0,self.height-190,'Od kodu źródłowego do programu natywnego')
        c.setFont('Sans',10.8)
        for i,s in enumerate(['Podręcznik koncepcji i implementacji',
                              'Przykłady · diagramy · przewodnik po źródłach']):
            c.drawString(0,self.height-228-i*18,s)
        labs=['TEKST','AST','HIR','LLVM IR','BINARKA']
        bw=(CW-4*13)/5
        y=self.height-332
        for i,lab in enumerate(labs):
            x=i*(bw+13)
            c.setStrokeColor(colors.HexColor('#72d2c2'))
            c.setFillColor(colors.HexColor('#1d4453'))
            c.roundRect(x,y,bw,47,5,fill=1,stroke=1)
            c.setFillColor(colors.white);c.setFont('SansB',9)
            c.drawCentredString(x+bw/2,y+19,lab)
            if i<4:
                c.line(x+bw,y+23,x+bw+13,y+23)
        c.setFillColor(colors.HexColor('#72d2c2'));c.setFont('SansB',10)
        c.drawString(0,62,'WYDANIE OPARTE NA MAIN · 56f4910')
        c.setFillColor(colors.white);c.setFont('Sans',9)
        c.drawString(0,42,'Stan kodu: 30 października 2026 · opracowanie: 2 listopada 2026')
        c.setFont('Sans',8)
        c.drawString(0,0,'Wersja polska  |  podręcznik do czytania i do wracania')

class Book(BaseDocTemplate):
    def __init__(self,path):
        super().__init__(str(path),pagesize=(W,H),leftMargin=M,rightMargin=M,
            topMargin=19*mm,bottomMargin=19*mm,
            title='Bork. Budowa kompilatora',author='Opracowanie na podstawie repozytorium Bork',
            subject=f'Podręcznik kompilatora Bork; main {SHA}',lang='pl-PL')
        self.chapter='';self.chapter_pages={};self.headings=[]
        self.addPageTemplates(PageTemplate('book',
            frames=Frame(M,19*mm,CW,H-38*mm,id='text',leftPadding=0,
                rightPadding=0,topPadding=0,bottomPadding=0),onPage=self.page_frame))
    def beforeDocument(self):
        self.chapter=''
        self.chapter_pages={};self.headings=[]
    def page_frame(self,c,doc):
        if doc.page==1:return
        c.saveState()
        c.setFillColor(INK);c.setFont('SansB',7.5)
        c.drawString(M,H-12*mm,'BORK / BUDOWA KOMPILATORA')
        c.setFont('Sans',7)
        c.drawRightString(W-M,H-12*mm,'Język · koncepcje · implementacja')
        c.setStrokeColor(colors.HexColor('#cbdadb'));c.setLineWidth(.4)
        c.line(M,15*mm,W-M,15*mm)
        c.setFont('Sans',7);c.setFillColor(TEAL)
        c.drawString(M,10*mm,'main 56f4910 · 30.10.2026')
        c.setFillColor(INK);c.setFont('SansB',9)
        c.drawRightString(W-M,10*mm,str(doc.page))
        c.restoreState()
    def afterFlowable(self,f):
        if hasattr(f,'bookmark'):
            text=f.getPlainText()
            self.canv.bookmarkPage(f.bookmark)
            self.canv.addOutlineEntry(text,f.bookmark,level=f.level,closed=f.level>0)
            self.headings.append((text,self.page,f.bookmark))
            if f.level==0:
                self.chapter=text
                self.chapter_pages[f.bookmark]=self.page
                self.notify('TOCEntry',(0,text,self.page,f.bookmark))

def make_figures(definitions):
    for key,body in definitions.items():
        dot='''digraph G {
graph [rankdir=TB, bgcolor="transparent", pad="0.12", nodesep="0.25", ranksep="0.35", fontname="DejaVu Sans", fontsize=11];
node [shape=box, style="rounded,filled", fillcolor="#edf4f4", color="#087e83", penwidth=1.1, fontname="DejaVu Sans", fontsize=11, margin="0.15,0.10"];
edge [color="#46636d", penwidth=1.1, arrowsize=0.7, fontname="DejaVu Sans", fontsize=9];
'''+body+'\n}'
        source=ROOT/'rysunki'/f'{key}.dot'
        svg=source.with_suffix('.svg')
        source.write_text(dot,encoding='utf-8')
        subprocess.run(['dot','-Tsvg',str(source),'-o',str(svg)],check=True,
                       capture_output=True)

def elements(markdown_text):
    raw=markdown.markdown(markdown_text,extensions=['fenced_code','tables','sane_lists'])
    return ET.fromstring('<root>'+raw+'</root>')

def build_story(text,definitions,reference_pages=None):
    story=[Cover(),PageBreak()]
    toc=TableOfContents()
    toc.levelStyles=[ParagraphStyle('toc',fontName='Sans',fontSize=10,
        leading=15,spaceBefore=8,textColor=INK,rightIndent=20)]
    story.extend([Paragraph('Spis treści',styles['h1']),toc,PageBreak()])
    chapter=0; section=0; figure=0
    for el in elements(text):
        if el.tag=='h1':
            chapter+=1;section=0;figure=0
            if story and not isinstance(story[-1],PageBreak):story.append(PageBreak())
            label=el.text or ''
            story.append(Paragraph('PODRĘCZNIK / '+ ('DODATEK' if not re.match(r'\d',label) else 'ROZDZIAŁ '+label.split('.')[0]),styles['small']))
            p=Paragraph(inline(el),styles['h1']);p.bookmark=f'c-{chapter}';p.level=0
            story.append(p)
        elif el.tag in ('h2','h3'):
            section+=1
            p=Paragraph(inline(el),styles[el.tag])
            p.bookmark=f's-{chapter}-{section}';p.level=1
            story.append(p)
        elif el.tag=='p':
            plain=''.join(el.itertext())
            m=re.fullmatch(r'\[\[diagram:([^|]+)\|([^|]+)\|(.+)\]\]',plain,re.S)
            if m:
                key,title,description=m.groups();figure+=1
                assert key in definitions,f'Brak rysunku {key}'
                drawing=VectorFigure(ROOT/'rysunki'/f'{key}.svg')
                caption=Paragraph(f'<b>Rysunek {chapter}.{figure}.</b> '+html.escape(title)+
                    '. '+html.escape(description),styles['caption'])
                story.append(KeepTogether([Spacer(1,6),drawing,caption]))
            elif plain=='[[indeks]]':
                terms=json.loads((ROOT/'indeks.json').read_text())
                for term,chapters in sorted(terms.items(),key=lambda x:x[0].lower()):
                    links=[]
                    for ch in chapters:
                        page=(reference_pages or {}).get(f'c-{ch}','…')
                        links.append(f'<a href="#c-{ch}" color="#087e83">{page}</a>')
                    story.append(Paragraph('<b>'+html.escape(term)+'</b> — '+', '.join(links),styles['small']))
            else:
                story.append(Paragraph(inline(el),styles['body']))
        elif el.tag=='pre':
            code=el.find('code')
            src=''.join(code.itertext()) if code is not None else ''.join(el.itertext())
            lang=(code.attrib.get('class','') if code is not None else '').replace('language-','')
            story.extend([Code(src.rstrip('\n'),lang),Spacer(1,9)])
        elif el.tag in ('ul','ol'):
            for n,li in enumerate(el.findall('li'),1):
                marker='•' if el.tag=='ul' else f'{n}.'
                story.append(Paragraph(marker+' '+inline(li),styles['bullet']))
        elif el.tag=='blockquote':
            for p in el:
                story.append(Paragraph(inline(p),styles['small']))
        elif el.tag=='table':
            rows=[]
            for tr in el.iter('tr'):
                rows.append([Paragraph(inline(c),styles['cell']) for c in tr])
            if not rows:continue
            n=max(map(len,rows))
            table=Table(rows,colWidths=[CW/n]*n,repeatRows=1,hAlign='LEFT')
            table.setStyle(TableStyle([
                ('BACKGROUND',(0,0),(-1,0),PALE),('LINEBELOW',(0,0),(-1,0),1,TEAL),
                ('LINEBELOW',(0,1),(-1,-1),.35,colors.HexColor('#d9e3e4')),
                ('VALIGN',(0,0),(-1,-1),'TOP'),('LEFTPADDING',(0,0),(-1,-1),6),
                ('RIGHTPADDING',(0,0),(-1,-1),6),('TOPPADDING',(0,0),(-1,-1),6),
                ('BOTTOMPADDING',(0,0),(-1,-1),6)]))
            story.extend([table,Spacer(1,10)])
        elif el.tag=='hr':
            story.append(Spacer(1,8))
        else:
            raise ValueError(f'Nieobsługiwany element {el.tag}')
    return story

def main():
    text=(ROOT/'podrecznik-sklad.md').read_text(encoding='utf-8')
    definitions=json.loads((ROOT/'rysunki.json').read_text(encoding='utf-8'))
    requested=re.findall(r'\[\[diagram:([^|]+)\|',text)
    assert len(requested)==len(set(requested)), 'Powtórzone identyfikatory rysunków'
    assert set(requested)==set(definitions), 'Zestaw rysunków różni się od treści'
    assert len(re.findall(r'^# ',text,re.M))>=24,'Brak rozdziałów'
    make_figures(definitions)
    target=ROOT/'Bork-budowa-kompilatora.pdf'
    pages={}
    # Trzeci przebieg sprawdza stabilność numerów indeksu po złożeniu spisu treści.
    for _ in range(3):
        doc=Book(target)
        doc.multiBuild(build_story(text,definitions,pages),maxPasses=6)
        if pages==doc.chapter_pages:break
        pages=doc.chapter_pages.copy()
    else:
        raise RuntimeError('Niestabilny indeks stron')
    (ROOT/'strony.json').write_text(json.dumps(doc.headings,ensure_ascii=False,indent=2))
    print(json.dumps({'pdf':str(target),'pages':doc.page,'figures':len(definitions),
        'words':len(text.split()),'bytes':target.stat().st_size},ensure_ascii=False))

if __name__=='__main__':
    main()
