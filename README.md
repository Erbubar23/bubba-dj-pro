# 🎧 BUBBA DJ PRO v0.5.0

Software DJ ligero de código abierto para Windows, escrito en Rust, con soporte nativo para **Pioneer DDJ-FLX4** (y la familia DDJ-400).

## Funciones

- **Dos decks** con formas de onda RGB de 3 bandas en alta resolución (HiDPI), rejilla de tiempos, marcadores de hot cue, loop, punto CUE y cabezal "fantasma" del modo Slip.
- **Análisis automático** al cargar, en dos pasos: el deck queda listo en < 1 s con la rejilla DSP y en 2-4 s se cambia por la rejilla precisa de la red neuronal **Beat This!** (ISMIR 2024): BPM sin errores de mitad/doble, compás 1 real y primer beat (CUE automático). Se guarda en caché: la segunda carga es instantánea. Tonalidad en notación **Camelot**.
- **Cue points sugeridos** (INICIO / DROP / BREAK / OUTRO) por análisis de energía alineado a frases de 4 compases, estilo análisis de frases de rekordbox.
- **Beat Sync** por rejilla: iguala el tempo (también a medio/doble tempo, 87 ↔ 174) y alinea la fase, y la mantiene corrigiendo la deriva. Medidor de fase entre decks en la barra superior. Dos modos, en el selector **SYNC** de la barra superior:
  - **rekordbox** (por defecto): el fader del deck sincronizado no actúa; el fader del master mueve ambos. Al apagar SYNC el tempo se queda y el fader retoma el control cuando lo llevas a la marca naranja (sin saltos).
  - **Serato 0%**: al sincronizar, el tempo sincronizado pasa a ser el nuevo 0% y el fader sube o baja desde ahí de inmediato, moviendo ambos decks. Al apagar SYNC sigue en modo relativo.
- **Reasignación del 0% estilo Serato** (botón 0% o Shift+SYNC): el tempo actual pasa a ser el nuevo 0% sin cambiar la velocidad.
- **Hot cues** (8 por deck; Shift+clic o Shift+pad para borrar), **auto loops cuantizados** (1/4 a 32 tiempos), Loop In/Out, Reloop, ÷2/×2, **Beat Jump** y **modo Slip**.
- **Mixer**: Trim, EQ de 3 bandas con kill, filtro **Color FX** bipolar, fader de canal, crossfader, nivel master y VU meters en dB.
- **Echo** sincronizado al tempo (1/4 a 2 tiempos), con cola que se apaga de forma natural.
- **"Stems"** en tiempo real (voz / batería / bajo, con solo). *Nota: es una aproximación por filtros, no una separación con IA.*
- **Enrutamiento de audio independiente**: Master a tu interfaz (ej. Behringer UMC) y pre-escucha a otra tarjeta (ej. la salida de 3.5 mm de tu PC), con compensación de deriva entre relojes. Si la salida master tiene 4 canales (la tarjeta interna de la FLX4), los audífonos salen por los canales 3-4.
- **Biblioteca**: navegador de carpetas (Música, Descargas, unidades…), búsqueda, orden por columnas, BPM/tono de las pistas ya analizadas, historial de sesión.
- **Arrastrar y soltar** desde la biblioteca o directamente desde el Explorador de Windows a cualquier deck.
- **Protección del deck al aire**: no deja cargar una pista sobre un deck que está sonando (mantén Shift para forzarlo).
- Formatos: MP3, WAV, FLAC, AIFF, M4A/AAC, ALAC, OGG.

## Uso

1. Conecta la DDJ-FLX4 por USB (se detecta y reconecta sola, también si la conectas con el programa abierto).
2. Abre `BubbaDJPro.exe` (o `Iniciar_Bubba_DJ_Pro.bat`). Se abre una ventana de aplicación; al cerrarla se cierra el programa.
3. Elige las salidas de audio en ⚙ **Configuración** (se recuerdan para la próxima vez).

Opcional: `BubbaDJPro.exe pista1.mp3 pista2.mp3` carga las pistas en los decks 1 y 2.

### Atajos de teclado

| Deck 1 | Deck 2 | Acción |
|---|---|---|
| `Q` | `U` | CUE (mantener) |
| `W` | `I` | Play / Pausa |
| `E` | `O` | Sync |
| `1`–`4` | `7`–`0` | Hot cues 1–4 (Shift = borrar) |
| `A` / `S` | `J` / `K` | Loop ÷2 / ×2 |
| `D` | `L` | Auto loop |

Biblioteca: `↑`/`↓` seleccionar, `Enter` abrir/cargar, `Shift+←`/`Shift+→` cargar en Deck 1/2, `Ctrl+F` buscar, `?` ayuda.

### DDJ-FLX4

Transporte, jog (pitch bend en el aro, scratch tocando el plato, búsqueda con Shift), tempo, Trim, EQ, filtro CFX, faders, crossfader, CUE de audífonos, sección de loop, pads (Hot Cue, Beat Jump, Beat Loop; en modo Sampler los pads 1-3 son los stems y el 4 es Slip), perilla de navegación y botones LOAD de la biblioteca.

## Arquitectura (módulos)

Cada sección vive en su propio módulo para poder trabajarla sin romper las demás:

| Crate / módulo | Qué hace |
|---|---|
| `bubba-analyzer/src/beatgrid/` | Rejilla: `onset` (envolventes), `dsp` (rápido), `fit` (rejilla constante desde beats), `neural` (Beat This!), `refine` (combina ambos) |
| `bubba-analyzer/src/cues.rs` | Cue points sugeridos |
| `bubba-core/src/tempo.rs` | Fader de tempo → velocidad (absoluto, relativo, pickup) |
| `bubba-core/src/sync.rs` | Beat sync y modos rekordbox / Serato |
| `bubba-app/src/analysis.rs` | Caché de análisis y mensajes a la interfaz |
| `bubba-app/src/loader.rs` | Carga de pistas: análisis rápido → deck → rejilla precisa |

Probar la rejilla con tus pistas: `cargo run --release -p bubba-analyzer --example grid_eval -- "pista1.mp3" "pista2.mp3"`

## Compilar

```powershell
cargo build --release      # genera target\release\bubba-app.exe
cargo test --workspace     # pruebas (las de hardware: cargo test -- --ignored)
```

Para depurar la interfaz en un navegador normal: `set BUBBA_NO_WINDOW=1` antes de ejecutar y abre `http://127.0.0.1:8080`.

Configuración, caché de análisis, modelo de beats y registro: `%APPDATA%\BubbaDJPro\`. Compilar requiere Rust 1.89 o superior (por `beat-this`).
