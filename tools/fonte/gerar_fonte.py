"""Fonte "Kage Display": letras dos dias da semana e o enfeite do horário, construídas do zero.

Cada letra sai de geometria: hastes, arcos de círculo ou de elipse (centro e raios), cortes de 45° e 60°,
todos sobre a grade da fonte (em 1000; maiúsculas de 800). Nenhum contorno é lido de outra fonte: o
arquivo nasce aqui (FontBuilder), com nome e métricas nossos.

A grade está em números inteiros de propósito: o DirectWrite do motor quantiza a cobertura vertical em
degraus, e uma borda 0,67 unidade fora do lugar já muda um degrau inteiro na tela.

Uso (precisa do fonttools: pip install fonttools):
    python tools/fonte/gerar_fonte.py assets/skins/Kage/KageDisplay.otf
"""
import math
import sys

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.t2CharStringPen import T2CharStringPen
from fontTools.feaLib.builder import addOpenTypeFeaturesFromString

FAMILIA = "Kage Display"
PS = "KageDisplay-Regular"

# ---------------------------------------------------------------- grade
H = 800       # altura das maiúsculas
TX = 133      # largura das hastes em pé
BASE = 134    # topo do traço de baixo
TOPO = 666    # base do traço de cima
M0, M1 = 333, 466   # barra do meio


def ang(cx, cy, rx, ry, p):
    """Ângulo paramétrico de um ponto na elipse."""
    return math.degrees(math.atan2((p[1] - cy) / ry, (p[0] - cx) / rx))


class Caminho:
    """Contorno de retas e arcos de elipse (o arco vira cúbicas de no máximo 45°)."""

    def __init__(self, p):
        self.ops = [("M", p)]
        self.cur = p

    def l(self, *p):
        p = p[0] if len(p) == 1 else p
        self.ops.append(("L", p))
        self.cur = p
        return self

    def arco(self, cx, cy, rx, ry, ate, sentido):
        """Do ponto atual até `ate` (ângulo em graus, ou um ponto da elipse); sentido +1 anti-horário."""
        alvo = ate if isinstance(ate, tuple) else None
        a0 = ang(cx, cy, rx, ry, self.cur)
        a1 = ang(cx, cy, rx, ry, alvo) if alvo else ate
        d = a1 - a0
        if sentido > 0:
            while d <= 1e-9:
                d += 360
        else:
            while d >= -1e-9:
                d -= 360
        n = max(1, math.ceil(abs(d) / 45 - 1e-9))
        passo = math.radians(d / n)
        k = 4 / 3 * math.tan(passo / 4)
        a = math.radians(a0)
        p0 = self.cur
        for i in range(n):
            b = a + passo
            p3 = (cx + rx * math.cos(b), cy + ry * math.sin(b))
            if alvo and i == n - 1:
                p3 = alvo
            p1 = (p0[0] - k * rx * math.sin(a), p0[1] + k * ry * math.cos(a))
            p2 = (p3[0] + k * rx * math.sin(b), p3[1] - k * ry * math.cos(b))
            self.ops.append(("C", (p1, p2, p3)))
            a, p0 = b, p3
        self.cur = p0
        return self


def poli(*pts):
    c = Caminho(pts[0])
    for p in pts[1:]:
        c.l(p)
    return c


def area(c):
    pts = [op[1] if op[0] != "C" else op[1][2] for op in c.ops]
    return sum(x0 * y1 - x1 * y0 for (x0, y0), (x1, y1) in zip(pts, pts[1:] + pts[:1])) / 2


def inverter(c):
    """O mesmo contorno no sentido contrário."""
    pontos = [c.ops[0][1]]
    segs = []
    for op in c.ops[1:]:
        if op[0] == "L":
            segs.append(("L", pontos[-1], op[1]))
            pontos.append(op[1])
        else:
            segs.append(("C", pontos[-1], op[1]))
            pontos.append(op[1][2])
    novo = Caminho(pontos[-1])
    for s in reversed(segs):
        if s[0] == "L":
            novo.l(s[1])
        else:
            p1, p2, _ = s[2]
            novo.ops.append(("C", (p2, p1, s[1])))
            novo.cur = s[1]
    return novo


# ---------------------------------------------------------------- letras

def letra_O():
    # anel: por fora 801 x 800, por dentro um círculo de 266 (traço de 134 em volta)
    fora = Caminho((400.5, H)).arco(400.5, 400, 400.5, 400, 90, +1)
    furo = Caminho((400, TOPO)).arco(400, 400, 266, 266, 90, -1)
    return [fora], [furo]


def letra_I():
    # haste com as duas pontas cortadas a 45°
    return [poli((0, 0), (TX, BASE), (TX, 668), (0, H))], []


def letra_D():
    cx, cy = 266, 400
    pe_dentro = 380  # a haste termina num corte de 45°, o lado de dentro mais alto
    c = (Caminho((0, 0)).l(cx, 0).arco(cx, cy, 400, 400, 90, +1).l(0, H).l(TX, TOPO).l(cx, TOPO)
         .arco(cx, cy, 267, 266, -90, -1).l(TX, BASE).l(TX, pe_dentro).l(0, pe_dentro - TX))
    return [c], []


def letra_M():
    # pernas de 132/133; o V do meio a 45°; a perna da esquerda termina num corte (dentro mais baixo)
    return [poli((0, 413), (TX, 280), (TX, 480), (401, 211), (669, 478), (669, 0), (801, 0), (801, H),
                 (401, 400), (0, H))], []


def letra_N():
    # diagonal de (0, 800) até a haste da direita em 321; perna da esquerda cortada como a do M
    return [poli((532, H), (532, 321), (0, H), (0, 413), (TX, 280), (TX, 478), (665, 0), (665, H))], []


def letra_S():
    # dois bojos: o de cima aberto à direita, o de baixo aberto à esquerda; braços cortados a 45°
    ca, cb = (233, 566.5), (432, 233)
    c = (Caminho((432, M0)).arco(432, 233.5, 101, 99.5, -90, -1).l(93, BASE).l(227, 0)
         .l(432, 0).arco(cb[0], cb[1], 234, 233, 90, +1).l(233, M1).arco(233, 566, 100, 100, 90, -1)
         .l(573, TOPO).l(439, H).l(233, H).arco(ca[0], ca[1], 233, 233.5, -90, +1))
    return [c], []


def letra_E():
    L = 533
    return [poli((0, 11), (0, 0), (L, 0), (L, BASE), (TX, BASE)),
            poli((0, H), (0, 450), (TX, 583), (TX, TOPO), (L, TOPO), (L, H)),
            poli((0, M0), (L, M0), (L, M1), (83, M1), (0, 383))], []


def letra_T():
    # barrinha solta à esquerda; haste de 134 e o braço da direita
    return [poli((0, H), (0, TOPO), (211, TOPO), (345, H)),
            poli((533, H), (399, TOPO), (399, 0), (533, 0), (533, TOPO), (799, TOPO), (799, H))], []


def letra_R():
    # bojo com centro em (351, 534) e furo de 133; perna solta a 60°; haste curta embaixo
    juncao = (453, 287)  # onde a borda direita da perna encontra o bojo
    c = (Caminho(juncao).arco(351, 534, 266, 266, 90, +1).l(0, H).l(69, 681).l(84, TOPO).l(351, TOPO)
         .arco(351, 533, 133, 133, -90, -1).l(232, 401).l(464, 0).l(617, 0))
    return [c, poli((0, 0), (TX, 0), (TX, 380), (0, 247))], []


def _anel_c(cx=401):
    # anel aberto à direita, pontas cortadas no raio de 45°
    return (Caminho((684, 682)).arco(cx, 400, 401, 400, (684, 117), +1).l(589, 212)
            .arco(cx, 400, 267, 266, (589, 589), -1))


def letra_C():
    return [_anel_c()], []


def letra_Ccedil():
    cx = 446
    ced = (Caminho((356, 60)).l(356, -130).l(cx, -130).arco(cx, -155, 25, 25, -90, -1).l(330, -180).l(420, -270)
           .l(cx, -270).arco(cx, -155, 115, 115, 90, +1).l(cx, 60))
    return [_anel_c(), ced], []


def letra_G():
    # anel aberto em cima à direita (corte a 45°); a ponta de baixo é um corte reto; barra no meio
    c = (Caminho((601, 224)).arco(400, 400, 266, 266, (588, 588), -1).l(684, 682)
         .arco(400, 400, 400, 400, (734, 180), +1))
    barra = poli((400, M0), (601, M0), (734, M1), (400, M1))
    return [c, barra], []


def letra_U():
    # meio anel embaixo; só a haste da esquerda sobe até o topo
    c = (Caminho((533, 333)).arco(333, 333, 200, 199, 180, -1).l(TX, H).l(0, H).l(0, 333)
         .arco(333, 333, 333, 333, 0, +1))
    return [c], []


def letra_Q():
    # anel com uma fenda a 45° na direita de baixo e a perna a 45° saindo por ela
    c = (Caminho((400, 0)).arco(400, 400, 400, 400, (632, 73), +1).l(705, 0).l(893, 0).l(603, 290)
         .l(415, 290).l(535, 170).arco(400, 401, 267, 267, (666, 374), -1).l(776, 264)
         .arco(400, 400, 400, 400, 270, +1))
    return [c], []


def _A(acento):
    # pernas inclinadas (2 de altura para 1 de largura); a da direita é interrompida entre 133 e 266,
    # onde entra o tracinho do meio, que sai da perna da esquerda com corte a 45°
    corpo = poli((450, 501), (567, 266), (717, 266), (450, H), (50, 0), (198, 0), (266, BASE), (348, BASE),
                 (480, 266), (332, 266))
    pe_ = poli((783, 133), (635, 133), (701, 0), (850, 0))
    cs = [corpo, pe_]
    if acento:
        cs.append(poli((365, 845), (485, 845), (595, 955), (475, 955)))
    return cs, []


def letra_B():
    # haste de cima cortada a 45°; dois bojos de 233 com furos de 100, que se encontram em (596, 400)
    c = (Caminho((0, H)).l(0, 450).l(TX, 583).l(TX, TOPO).l(433, TOPO).arco(433, 566, 100, 100, -90, -1)
         .l(83, M1).l(0, 383).l(0, M0).l(433, M0).arco(433, 233.5, 100, 99.5, -90, -1)
         .l(TX, BASE).l(TX, 0).l(433, 0).arco(433, 233, 233, 233, (596, 400), +1)
         .arco(433, 567, 233, 233, 90, +1))
    return [c], []


def letra_X():
    # quatro braços que se encontram num losango no meio
    esq, dir_, cima, baixo = (314, 400), (483, 400), (398, 508), (398, 292)
    return [poli((171, H), (0, H), esq, cima), poli((0, 0), (171, 0), baixo, esq),
            poli((795, H), (626, H), cima, dir_), poli(baixo, (626, 0), (795, 0), dir_)], []


def enfeite():
    """O enfeite "em S" em volta do horário (desenho nosso, de 15/11/2026)."""
    direita = poli((2805, -142), (2750, -142), (2805, -198), (4691, -198), (5200, 311), (5200, 300), (5800, 300),
                   (5700, 400), (5200, 400), (5200, 378), (5189, 378), (4669, -142))
    esquerda = poli((400, 300), (900, 300), (900, 322), (911, 322), (1431, 842), (3350, 842), (3295, 898),
                    (1409, 898), (900, 389), (900, 400), (300, 400))
    return [direita, esquerda], []


def notdef():
    return [poli((50, 0), (450, 0), (450, 700), (50, 700))], [poli((100, 50), (100, 650), (400, 650), (400, 50))]


GLIFOS = [
    # nome, unicode, avanço, desenho
    (".notdef", None, 500, notdef),
    ("space", 0x20, 250, None),
    ("A", 0x41, 899, lambda: _A(False)),
    ("B", 0x42, 809, letra_B),
    ("C", 0x43, 720, letra_C),
    ("D", 0x44, 830, letra_D),
    ("E", 0x45, 636, letra_E),
    ("G", 0x47, 875, letra_G),
    ("I", 0x49, 140, letra_I),
    ("M", 0x4D, 954, letra_M),
    ("N", 0x4E, 818, letra_N),
    ("O", 0x4F, 965, letra_O),
    ("Q", 0x51, 962, letra_Q),
    ("R", 0x52, 761, letra_R),
    ("S", 0x53, 803, letra_S),
    ("T", 0x54, 909, letra_T),
    ("U", 0x55, 782, letra_U),
    ("X", 0x58, 919, letra_X),
    ("bar", 0x7C, 6100, enfeite),
    ("Aacute", 0xC1, 899, lambda: _A(True)),
    ("Ccedilla", 0xC7, 720, letra_Ccedil),
]

# Pares em GPOS (o DirectWrite do motor só aplica o kern por GPOS)
PARES = [("C", "A", -170), ("Ccedilla", "A", -170), ("D", "A", -100), ("T", "A", -190)]

# Dicas de haste (hinting) das letras de traço redondo ou cheio: (horizontais, verticais), cada uma (de, até).
# Fantasma na base = (21, 0); no topo = (800, 780). O DirectWrite encaixa essas bordas no pixel.
_FB, _FT = (21, 0), (800, 780)
DICAS = {
    "O": ([(0, BASE), (TOPO, H)], [(0, BASE), (TOPO, 801)]),
    "G": ([(0, BASE), (M0, M1), (TOPO, H)], [(0, BASE)]),
    "U": ([(0, BASE), _FT], [(0, TX), (533, 666)]),
    "I": ([_FB, _FT], [(0, TX)]),
    "X": ([_FB, _FT], [(0, 795)]),
}


def _hastes(lista, operador):
    args, antes = [], 0
    for de, ate in lista:
        args += [de - antes, ate - de]
        antes = ate
    return args + [operador]


def desenhar(nome, fn, avanco):
    """Devolve a charstring (com avanço e dicas) e o x mais à esquerda do desenho (para o lsb)."""
    from fontTools.pens.boundsPen import BoundsPen
    pen = T2CharStringPen(None, None)
    bp = BoundsPen(None)
    if fn is None:
        cs = pen.getCharString()
        cs.program = [avanco] + cs.program
        return cs, 0
    fora, furos = fn()
    for c in fora + furos:
        quer_positivo = c in fora
        if (area(c) > 0) != quer_positivo:
            c = inverter(c)
        for alvo in (pen, bp):
            alvo.moveTo(tuple(round(v, 2) for v in c.ops[0][1]))
            for op in c.ops[1:]:
                if op[0] == "L":
                    alvo.lineTo(tuple(round(v, 2) for v in op[1]))
                else:
                    alvo.curveTo(*[tuple(round(v, 2) for v in p) for p in op[1]])
            alvo.closePath()
    cs = pen.getCharString()
    dicas = []
    if nome in DICAS:
        h, v = DICAS[nome]
        dicas = _hastes(h, "hstem") + _hastes(v, "vstem")
    cs.program = [avanco] + dicas + cs.program
    return cs, round(bp.bounds[0])


def main(saida):
    ordem = [g[0] for g in GLIFOS]
    fb = FontBuilder(1000, isTTF=False)
    fb.setupGlyphOrder(ordem)
    fb.setupCharacterMap({g[1]: g[0] for g in GLIFOS if g[1] is not None})
    feitos = {g[0]: desenhar(g[0], g[3], g[2]) for g in GLIFOS}
    cs = {n: v[0] for n, v in feitos.items()}
    # Zonas de alinhamento: linha de base e altura das maiúsculas; hastes de T
    privado = {"BlueValues": [0, 0, 800, 800], "StdHW": 134, "StdVW": 133}
    fb.setupCFF(PS, {"FullName": FAMILIA + " Regular", "FamilyName": FAMILIA, "Weight": "Regular"}, cs, privado)
    metricas = {g[0]: (g[2], feitos[g[0]][1]) for g in GLIFOS}
    fb.setupHorizontalMetrics(metricas)
    fb.setupHorizontalHeader(ascent=900, descent=-300, lineGap=0)
    fb.setupNameTable({
        "copyright": "Copyright 2026 LucasDias777. Parte do Kage (github.com/LucasDias777/Kage).",
        "familyName": FAMILIA,
        "styleName": "Regular",
        "uniqueFontIdentifier": "Kage:" + PS,
        "fullName": FAMILIA + " Regular",
        "psName": PS,
        "version": "Version 1.000",
        "licenseDescription": "PolyForm Noncommercial 1.0.0, a mesma licença do Kage.",
    })
    fb.setupOS2(version=3, usWeightClass=400, usWidthClass=5, fsType=0, fsSelection=0x40, sxHeight=400,
                sCapHeight=800, usWinAscent=1000, usWinDescent=200, sTypoAscender=800, sTypoDescender=-200,
                sTypoLineGap=200, achVendID="KAGE", ulCodePageRange1=1)
    fb.setupPost(underlinePosition=-75, underlineThickness=50)
    fea = "languagesystem DFLT dflt;\nlanguagesystem latn dflt;\nfeature kern {\n" + "".join(
        f"  pos {a} {b} {v};\n" for a, b, v in PARES) + "} kern;\n"
    addOpenTypeFeaturesFromString(fb.font, fea)
    fb.font["head"].fontRevision = 1.0
    fb.save(saida)
    print("ok", saida)


if __name__ == "__main__":
    main(sys.argv[1])
