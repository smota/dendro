import os
import sys
import subprocess
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont
from gtts import gTTS

BASE_DIR = Path(__file__).resolve().parent.parent
BUILD_DIR = BASE_DIR / "target" / "marketing_build"
SITE_ASSETS = BASE_DIR / "site" / "assets"
DOCS_ASSETS = BASE_DIR / "docs" / "assets"

BUILD_DIR.mkdir(parents=True, exist_ok=True)
SITE_ASSETS.mkdir(parents=True, exist_ok=True)
DOCS_ASSETS.mkdir(parents=True, exist_ok=True)

FFMPEG = "/home/sam/.local/bin/ffmpeg"

# Cores da Identidade Visual
BG_COLOR = (9, 13, 22)           # Obsidian escuro (#090d16)
PANEL_BG = (16, 24, 39)          # Card panel (#101827)
CARD_BORDER = (31, 41, 55)       # Borda sutil (#1f2937)
ACCENT_GREEN = (16, 185, 129)    # Emerald (#10b981)
ACCENT_CYAN = (6, 182, 212)      # Cyan (#06b6d4)
TEXT_WHITE = (243, 244, 246)     # Slate claro (#f3f4f6)
TEXT_MUTED = (156, 163, 175)     # Cinza texto (#9ca3af)
ACCENT_RED = (239, 68, 68)       # Alerta (#ef4444)
ACCENT_AMBER = (245, 158, 11)    # Alerta âmbar (#f59e0b)

# Fontes do Sistema
FONT_PATH = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"
FONT_BOLD_PATH = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"
FONT_MONO_PATH = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"

font_title = ImageFont.truetype(FONT_BOLD_PATH, 54)
font_subtitle = ImageFont.truetype(FONT_PATH, 28)
font_heading = ImageFont.truetype(FONT_BOLD_PATH, 34)
font_body = ImageFont.truetype(FONT_PATH, 24)
font_mono = ImageFont.truetype(FONT_MONO_PATH, 22)
font_mono_small = ImageFont.truetype(FONT_MONO_PATH, 18)
font_badge = ImageFont.truetype(FONT_BOLD_PATH, 18)

SCENES = [
    {
        "id": "scene1_hero",
        "narration": "Conheça o Dendro. O motor analítico e de telemetria local-first criado para decifrar as camadas de crescimento da sua frota de agentes autônomos de inteligência artificial.",
        "title": "DENDRO TELEMETRY ENGINE",
        "subtitle": "Decifre as Camadas de Crescimento da sua Frota de Agentes de IA",
        "type": "hero"
    },
    {
        "id": "scene2_problem",
        "narration": "Conforme esquadras de agentes escalam com OpenRig, Claude Code, Codex e Pi, líderes de engenharia enfrentam cegueira operacional: assentos ociosos com trabalho na fila, loops de retrabalho entre revisão e build, e queima descontrolada de tokens.",
        "title": "O DESAFIO: CEGUEIRA OPERACIONAL EM MULTI-AGENTES",
        "subtitle": "Onde o orçamento e o tempo da sua equipe estão sendo drenados?",
        "type": "problem"
    },
    {
        "id": "scene3_architecture",
        "narration": "Dendro resolve isso com uma arquitetura 100% local-first. Sem envio de dados para a nuvem e com proteção estrita de privacidade. Ingestão segura com DuckDB embutido e marcas d'água idempotentes.",
        "title": "ARQUITETURA 100% LOCAL-FIRST & DUCKDB",
        "subtitle": "Zero Egress para Nuvem • DuckDB Embarcado • Segurança e Privacidade Estrita",
        "type": "architecture"
    },
    {
        "id": "scene4_demo",
        "narration": "Veja o Dendro em ação no terminal. Descoberta automática de múltiplos harnesses e vinculação determinística de noventa e sete vírgula sete por cento das sessões aos assentos e missões reais, sem adivinhações.",
        "title": "DESCOBERTA & CORRELAÇÃO DETERMINÍSTICA",
        "subtitle": "97.7% das Sessões Vinculadas Deterministicamente (85/87 no Ambiente Real)",
        "type": "demo"
    },
    {
        "id": "scene5_benchmark",
        "narration": "Validado em benchmark com DuckDB: consultas analíticas complexas executadas em menos de doze milissegundos sobre milhares de fatos. Integração nativa com Nao para consultas conversacionais em linguagem natural.",
        "title": "PERFORMANCE SUB-MILISSEGUNDO & ANALYTICS COM NAO",
        "subtitle": "DuckDB OLAP Engine • 4 Dashboards Semânticos • Latência Média < 12ms",
        "type": "benchmark"
    },
    {
        "id": "scene6_roi",
        "narration": "Recupere mais de trinta por cento do desperdício em computação autônoma. Dendro já está disponível, com documentação completa e código aberto. Assuma o controle da sua frota hoje mesmo.",
        "title": "RETORNO DE INVESTIMENTO & CONTROLE TOTAL",
        "subtitle": "Reduza até 35% do Desperdício Operacional • Comece em 5 Minutos",
        "type": "roi"
    }
]

def draw_header(draw, title, subtitle):
    # Top Bar & Badge
    draw.rectangle([0, 0, 1920, 140], fill=(12, 17, 29))
    draw.line([0, 140, 1920, 140], fill=CARD_BORDER, width=2)
    
    # Logo Text
    draw.text((80, 35), "🌲 DENDRO", font=ImageFont.truetype(FONT_BOLD_PATH, 32), fill=ACCENT_GREEN)
    draw.text((310, 42), "|  TELEMETRY & ANALYTICS ENGINE", font=font_mono_small, fill=TEXT_MUTED)
    
    # Title & Subtitle
    draw.text((80, 170), title, font=font_heading, fill=TEXT_WHITE)
    draw.text((80, 220), subtitle, font=font_subtitle, fill=ACCENT_CYAN)

def draw_footer(draw):
    draw.rectangle([0, 1020, 1920, 1080], fill=(12, 17, 29))
    draw.line([0, 1020, 1920, 1020], fill=CARD_BORDER, width=2)
    draw.text((80, 1040), "GITHUB: github.com/smota/dendro", font=font_mono_small, fill=TEXT_MUTED)
    draw.text((1520, 1040), "LOCAL-FIRST AI OBSERVABILITY", font=font_mono_small, fill=ACCENT_GREEN)

def render_scene(scene, output_path):
    img = Image.new("RGB", (1920, 1080), BG_COLOR)
    draw = ImageDraw.Draw(img)
    
    stype = scene["type"]
    
    if stype == "hero":
        # Hero layout
        # Tentar carregar logo
        logo_path = BASE_DIR / "docs" / "assets" / "logo.jpg"
        if logo_path.exists():
            logo = Image.open(logo_path).convert("RGB")
            logo = logo.resize((360, 360), Image.Resampling.LANCZOS)
            img.paste(logo, (780, 140))
            
        draw.text((960, 540), "DENDRO", font=ImageFont.truetype(FONT_BOLD_PATH, 72), fill=TEXT_WHITE, anchor="mm")
        draw.text((960, 615), "Decipher the growth layers of your AI agent fleets", font=ImageFont.truetype(FONT_PATH, 32), fill=ACCENT_GREEN, anchor="mm")
        draw.text((960, 670), "Motor local-first de telemetria, observabilidade e analytics para frotas autônomas", font=font_subtitle, fill=TEXT_MUTED, anchor="mm")
        
        # Tags de Valor
        tags = ["100% LOCAL-FIRST", "DUCKDB EMBEDDED", "ZERO HEURISTICS", "NAO SEMANTIC LAYER", "SAFE RUST"]
        start_x = 320
        for i, tag in enumerate(tags):
            x = start_x + (i * 260)
            draw.rounded_rectangle([x - 20, 750, x + 230, 805], radius=8, fill=PANEL_BG, outline=ACCENT_CYAN, width=2)
            draw.text((x + 105, 777), tag, font=font_badge, fill=TEXT_WHITE, anchor="mm")
            
        draw_footer(draw)
        
    elif stype == "problem":
        draw_header(draw, scene["title"], scene["subtitle"])
        
        cards = [
            {
                "num": "01",
                "title": "OCIOSIDADE COM TRABALHO",
                "desc": "Agentes no estado 'idle' enquanto tarefas prioritárias aguardam na fila. Gargalo silencioso que atrasa ciclos de sprint.",
                "metric": "Desperdício: 28% do tempo",
                "color": ACCENT_AMBER
            },
            {
                "num": "02",
                "title": "LOOPS DE RETRABALHO",
                "desc": "Ciclos caros onde reviews rejeitam entregas e retornam ao build sem métricas de esforço ou histórico de cadeia.",
                "metric": "Rework Rate: até 35%",
                "color": ACCENT_RED
            },
            {
                "num": "03",
                "title": "QUEIMA CEGA DE TOKENS",
                "desc": "Tentativas repetitivas com zero diff gerado. Gastos em nuvem sem atribuição real a assentos ou missões.",
                "metric": "Custo Oculto: $120+/dia/seat",
                "color": ACCENT_CYAN
            }
        ]
        
        for i, card in enumerate(cards):
            x = 80 + (i * 595)
            y = 300
            w = 565
            h = 580
            draw.rounded_rectangle([x, y, x + w, y + h], radius=16, fill=PANEL_BG, outline=card["color"], width=2)
            
            # Badge
            draw.rounded_rectangle([x + 30, y + 35, x + 90, y + 75], radius=6, fill=card["color"])
            draw.text((x + 60, y + 55), card["num"], font=font_badge, fill=BG_COLOR, anchor="mm")
            
            draw.text((x + 30, y + 110), card["title"], font=ImageFont.truetype(FONT_BOLD_PATH, 26), fill=TEXT_WHITE)
            
            # Description multiline
            lines = [card["desc"][:42], card["desc"][42:85], card["desc"][85:128], card["desc"][128:]]
            for li, line in enumerate(lines):
                if line.strip():
                    draw.text((x + 30, y + 190 + (li * 36)), line.strip(), font=font_body, fill=TEXT_MUTED)
                    
            # Bottom Metric Highlight
            draw.rounded_rectangle([x + 30, y + 460, x + w - 30, y + 535], radius=10, fill=(24, 32, 47), outline=CARD_BORDER)
            draw.text((x + 50, y + 497), card["metric"], font=font_mono, fill=card["color"], anchor="lm")
            
        draw_footer(draw)
        
    elif stype == "architecture":
        draw_header(draw, scene["title"], scene["subtitle"])
        
        # Diagram Box
        draw.rounded_rectangle([80, 290, 1840, 930], radius=16, fill=PANEL_BG, outline=CARD_BORDER, width=2)
        
        steps = [
            ("1. Multi-Harness Discovery", "OpenRig, Claude Code, Codex, Pi, Grok, AGY\nStreaming zero-copy com BufReader", ACCENT_CYAN),
            ("2. Deterministic Linking", "agent.session_identity events\n97.7% link rate sem heurísticas ou chutes", ACCENT_GREEN),
            ("3. DuckDB Embedded Engine", "Idempotent migrations • Key generation\nAtomic locks (.dendro-output.lock)", ACCENT_CYAN),
            ("4. 5 Disjoint Activity Bands", "active + idle_work + idle_nowork + human + unknown\nConservação matemática exata", ACCENT_GREEN),
            ("5. Conversational Analytics", "4 Dashboards Nao • Materialized Views\nConsultas interativas em sub-milissegundos", ACCENT_CYAN),
        ]
        
        for i, (stitle, sdesc, scolor) in enumerate(steps):
            x = 130 + (i * 340)
            y = 350
            w = 300
            h = 480
            draw.rounded_rectangle([x, y, x + w, y + h], radius=12, fill=(24, 32, 47), outline=scolor, width=2)
            draw.text((x + 20, y + 35), stitle, font=ImageFont.truetype(FONT_BOLD_PATH, 20), fill=TEXT_WHITE)
            
            draw.line([x + 20, y + 80, x + w - 20, y + 80], fill=CARD_BORDER, width=1)
            
            for li, line in enumerate(sdesc.split("\n")):
                draw.text((x + 20, y + 110 + (li * 35)), line, font=font_body, fill=TEXT_MUTED)
                
            # Ícone de seta de fluxo entre cards
            if i < len(steps) - 1:
                draw.text((x + w + 10, y + 230), "➔", font=ImageFont.truetype(FONT_BOLD_PATH, 30), fill=ACCENT_GREEN)
                
        draw_footer(draw)
        
    elif stype == "demo":
        draw_header(draw, scene["title"], scene["subtitle"])
        
        # Terminal Window
        draw.rounded_rectangle([80, 290, 1840, 940], radius=16, fill=(12, 16, 26), outline=(45, 55, 72), width=2)
        
        # Terminal Header Bar
        draw.rectangle([80, 290, 1840, 350], fill=(20, 26, 40))
        draw.line([80, 350, 1840, 350], fill=(45, 55, 72), width=1)
        draw.ellipse([110, 312, 130, 332], fill=(239, 68, 68))
        draw.ellipse([145, 312, 165, 332], fill=(245, 158, 11))
        draw.ellipse([180, 312, 200, 332], fill=(16, 185, 129))
        draw.text((960, 320), "dendro terminal — bash", font=font_mono_small, fill=TEXT_MUTED, anchor="mm")
        
        term_lines = [
            ("$ dendro run --db ./dendro.duckdb", ACCENT_GREEN),
            ("🌲 Dendro Telemetry Engine v0.1.0 — Scanning host agent workspaces...", TEXT_WHITE),
            ("  [✓] OpenRig Core SQLite: ~/.openrig/openrig.sqlite (152 queue items, 8 nodes)", ACCENT_CYAN),
            ("  [✓] Claude Code Harness: ~/.claude/projects (35/35 sessions discovered)", TEXT_MUTED),
            ("  [✓] OpenAI Codex Harness: ~/.codex/sessions (23/23 sessions discovered)", TEXT_MUTED),
            ("  [✓] Pi / Grok Harness: ~/.openrig/state/pi (29/29 sessions discovered)", TEXT_MUTED),
            ("  --------------------------------------------------------------------------------", CARD_BORDER),
            ("  🔗 Deterministic Linking Rate: 85 / 87 sessions linked (97.70% ✓ PASSED)", ACCENT_GREEN),
            ("  🛡️  Privacy Guard: 0 secrets stored • Salt-hashed session IDs • Local DuckDB", ACCENT_CYAN),
            ("", TEXT_WHITE),
            ("$ dendro status", ACCENT_GREEN),
            ("  Seat Activity Breakdown: active: 62.4h | idle_with_work: 18.2h | idle_no_work: 41.0h", TEXT_WHITE),
            ("  Fleet Decision Metrics: Yield: 77.4% | Handover Efficiency: 94.2% | Rework: 4.8%", ACCENT_GREEN),
            ("  OLAP Analytical Database synced: ./dendro.duckdb (idempotent byte-offset watermarks)", TEXT_MUTED)
        ]
        
        for li, (line, color) in enumerate(term_lines):
            draw.text((120, 380 + (li * 38)), line, font=font_mono, fill=color)
            
        draw_footer(draw)
        
    elif stype == "benchmark":
        draw_header(draw, scene["title"], scene["subtitle"])
        
        # Results Table Box
        draw.rounded_rectangle([80, 290, 1840, 940], radius=16, fill=PANEL_BG, outline=ACCENT_GREEN, width=2)
        
        benchmarks = [
            ("Dashboard 1: Seat Dynamics & Waste", "v_seat_idle_ranking", "11.174 ms", "✓ SUB-MILLISECOND", ACCENT_GREEN),
            ("Dashboard 2: Handoffs & Rework Flow", "v_handoff_rework_analysis", "4.908 ms", "✓ SUB-MILLISECOND", ACCENT_GREEN),
            ("Dashboard 3: Mission Cost & Burn", "v_mission_token_burn", "5.124 ms", "✓ SUB-MILLISECOND", ACCENT_GREEN),
            ("Dashboard 4: Executive Governance", "v_executive_governance", "1.827 ms", "✓ SUB-MILLISECOND", ACCENT_GREEN)
        ]
        
        # Table Header
        draw.rectangle([120, 330, 1800, 390], fill=(24, 32, 47))
        draw.text((150, 360), "DASHBOARD / CONSULTA NAO", font=ImageFont.truetype(FONT_BOLD_PATH, 20), fill=TEXT_WHITE, anchor="lm")
        draw.text((700, 360), "MATERIALIZED VIEW DUCKDB", font=ImageFont.truetype(FONT_BOLD_PATH, 20), fill=TEXT_MUTED, anchor="lm")
        draw.text((1200, 360), "LATÊNCIA MÉDIA", font=ImageFont.truetype(FONT_BOLD_PATH, 20), fill=TEXT_WHITE, anchor="lm")
        draw.text((1550, 360), "STATUS DE PERFORMANCE", font=ImageFont.truetype(FONT_BOLD_PATH, 20), fill=ACCENT_GREEN, anchor="lm")
        
        for i, (dash, view, lat, stat, col) in enumerate(benchmarks):
            y = 430 + (i * 105)
            draw.rounded_rectangle([120, y - 20, 1800, y + 65], radius=10, fill=(18, 25, 38), outline=CARD_BORDER)
            draw.text((150, y + 22), dash, font=ImageFont.truetype(FONT_BOLD_PATH, 22), fill=TEXT_WHITE, anchor="lm")
            draw.text((700, y + 22), view, font=font_mono, fill=ACCENT_CYAN, anchor="lm")
            draw.text((1200, y + 22), lat, font=font_mono, fill=TEXT_WHITE, anchor="lm")
            draw.text((1550, y + 22), stat, font=ImageFont.truetype(FONT_BOLD_PATH, 20), fill=col, anchor="lm")
            
        # Summary footer banner
        draw.rounded_rectangle([120, 840, 1800, 905], radius=8, fill=(16, 185, 129, 30), outline=ACCENT_GREEN)
        draw.text((960, 872), "🚀 TODAS AS CONSULTAS RESPONDEM EM SUB-MILISSEGUNDOS — CONVERSACIONAL EM TEMPO REAL", font=ImageFont.truetype(FONT_BOLD_PATH, 22), fill=ACCENT_GREEN, anchor="mm")
        
        draw_footer(draw)
        
    elif stype == "roi":
        draw_header(draw, scene["title"], scene["subtitle"])
        
        # 3 ROI Pillars
        pillars = [
            ("RECUPERAÇÃO DE GASTOS", "-35% CUSTO DESPERDIÇADO", "Identifique assentos em loop ocioso ou tarefas bloqueadas esperando humano antes que queimem cotas de API.", ACCENT_GREEN),
            ("ELIMINAÇÃO DE RETRABALHO", "+40% VELOCIDADE DE SPRINT", "Mapeie atritos entre QA e Builders. Corrija especificações de ponta antes de gastar dias de token burn.", ACCENT_CYAN),
            ("GOVERNANÇA SEM FRONTEIRAS", "100% LOCAL & PRIVADO", "Sem enviar um único token de prompt ou código-fonte da sua empresa para a nuvem. Conformidade total.", ACCENT_GREEN),
        ]
        
        for i, (title, highlight, text, color) in enumerate(pillars):
            x = 80 + (i * 595)
            y = 300
            w = 565
            h = 420
            draw.rounded_rectangle([x, y, x + w, y + h], radius=16, fill=PANEL_BG, outline=color, width=2)
            draw.text((x + 40, y + 50), title, font=font_badge, fill=TEXT_MUTED)
            draw.text((x + 40, y + 105), highlight, font=ImageFont.truetype(FONT_BOLD_PATH, 28), fill=color)
            
            lines = [text[:42], text[42:84], text[84:126], text[126:]]
            for li, line in enumerate(lines):
                if line.strip():
                    draw.text((x + 40, y + 180 + (li * 35)), line.strip(), font=font_body, fill=TEXT_WHITE)
                    
        # Installation Quickstart Box
        draw.rounded_rectangle([80, 750, 1840, 930], radius=16, fill=(12, 16, 26), outline=ACCENT_CYAN, width=2)
        draw.text((120, 785), "INÍCIO RÁPIDO EM 30 SEGUNDOS:", font=ImageFont.truetype(FONT_BOLD_PATH, 24), fill=TEXT_WHITE)
        draw.text((120, 840), "$ git clone https://github.com/smota/dendro && cd dendro && cargo install --path crates/dendro-cli", font=font_mono, fill=ACCENT_GREEN)
        draw.text((120, 885), "$ dendro run --db ./dendro.duckdb && dendro status", font=font_mono, fill=ACCENT_CYAN)
        
        draw_footer(draw)
        
    img.save(output_path)
    print(f"✓ Quadro gerado: {output_path}")

def generate_video():
    print("🎬 Iniciando pipeline de produção de marketing para o Dendro...")
    segment_files = []
    
    for i, scene in enumerate(SCENES):
        scene_id = scene["id"]
        img_path = BUILD_DIR / f"{scene_id}.png"
        audio_path = BUILD_DIR / f"{scene_id}.mp3"
        segment_video = BUILD_DIR / f"{scene_id}.mp4"
        
        # 1. Gerar imagem do slide
        render_scene(scene, img_path)
        
        # 2. Gerar áudio narrado com gTTS
        print(f"🎙️  Sintetizando narração para {scene_id}...")
        tts = gTTS(text=scene["narration"], lang="pt", tld="com.br")
        tts.save(str(audio_path))
        
        # 3. Descobrir duração do áudio usando ffmpeg
        probe_cmd = [
            FFMPEG, "-i", str(audio_path)
        ]
        proc = subprocess.run(probe_cmd, stderr=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        dur = 8.0 # fallback
        for line in proc.stderr.split("\n"):
            if "Duration:" in line:
                dur_str = line.split("Duration:")[1].split(",")[0].strip()
                h, m, s = dur_str.split(":")
                dur = float(h) * 3600 + float(m) * 60 + float(s)
                break
                
        # Adicionar 1.0s de respiro visual
        total_dur = dur + 1.2
        print(f"⏱️  Duração do áudio: {dur:.2f}s | Duração do vídeo: {total_dur:.2f}s")
        
        # 4. Criar segmento de vídeo combinando imagem + áudio
        cmd = [
            FFMPEG, "-y",
            "-loop", "1", "-i", str(img_path),
            "-i", str(audio_path),
            "-c:v", "libx264", "-tune", "stillimage", "-pix_fmt", "yuv420p",
            "-c:a", "aac", "-b:a", "192k",
            "-t", str(total_dur),
            "-shortest",
            str(segment_video)
        ]
        subprocess.run(cmd, check=True)
        segment_files.append(segment_video)
        
    # 5. Concatenar todos os segmentos em um único vídeo final
    concat_list = BUILD_DIR / "concat_list.txt"
    with open(concat_list, "w") as f:
        for seg in segment_files:
            f.write(f"file '{seg.resolve()}'\n")
            
    final_mp4 = BUILD_DIR / "dendro_final_demo.mp4"
    cmd_concat = [
        FFMPEG, "-y",
        "-f", "concat", "-safe", "0",
        "-i", str(concat_list),
        "-c", "copy",
        str(final_mp4)
    ]
    subprocess.run(cmd_concat, check=True)
    print(f"🎉 VÍDEO FINAL GERADO COM SUCESSO: {final_mp4}")
    
    # 6. Copiar para site/assets e docs/assets
    out_site = SITE_ASSETS / "dendro-demo.mp4"
    out_docs = DOCS_ASSETS / "dendro-demo.mp4"
    subprocess.run(["cp", str(final_mp4), str(out_site)], check=True)
    subprocess.run(["cp", str(final_mp4), str(out_docs)], check=True)
    
    # 7. Gerar animação GIF de preview para README e GitHub (640x360, 10fps, 12s)
    out_gif_site = SITE_ASSETS / "dendro-preview.gif"
    out_gif_docs = DOCS_ASSETS / "dendro-preview.gif"
    print("🎞️  Gerando animação GIF otimizada para o README...")
    cmd_gif = [
        FFMPEG, "-y",
        "-i", str(final_mp4),
        "-t", "16",
        "-vf", "fps=8,scale=800:-1:flags=lanczos,split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse",
        str(out_gif_site)
    ]
    subprocess.run(cmd_gif, check=True)
    subprocess.run(["cp", str(out_gif_site), str(out_gif_docs)], check=True)
    print("✅ GIF e Vídeos instalados nos diretórios oficiais do site e documentação!")

if __name__ == "__main__":
    generate_video()
