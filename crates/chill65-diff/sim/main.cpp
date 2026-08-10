// Verilated MiSTer Crystal Castles core, driven headless.
//
// Our own work; the RTL in Arcade-CrystalCastles_MiSTer is third-party
// reference material and is never modified. Everything verilator needs is
// passed as a flag.
//
//   argv[1]  ROM blob: the seven devices concatenated in dn_addr order
//   argv[2]  output file for raw frames
//   argv[3]  frames to capture
//   argv[4]  optional input file, one line per frame: "<IN0 mask> <x> <y>"
//
// Writes a sidecar "<out>.dims" with "<width> <height> <emitted>", where
// `emitted` is how many pixels per line the core actually produced. See the
// note on the observable window below.

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>
#include <string>

#include "VCCastles.h"
#include "verilated.h"

// The harness's canonical frame.
static const int WIDTH = 256;
static const int HEIGHT = 232;

// The core's ROM download port takes 8K per device, selected by dn_addr[15:13],
// in this order (ProgramMemory.v and MotionObjectPictureRom.v):
//   0 ic1F  1 ic1H  2 ic8D  3 ic8B  4 ic1K  5 ic1L  6 ic1N
static const int DEVICES = 7;
static const int DEVICE_BYTES = 8192;

struct Input {
    unsigned sw, x, y;
};

// 3-bit component to 8-bit, matching cram_rgb's scaling exactly so the two
// implementations are directly comparable.
static inline unsigned char scale3(unsigned v) {
    return (unsigned char)((v * 255) / 7);
}

int main(int argc, char** argv) {
    if (argc < 4) {
        fprintf(stderr, "usage: mistersim <roms.bin> <out.raw> <frames> [input.txt]\n");
        return 2;
    }
    const char* rom_path = argv[1];
    const char* out_path = argv[2];
    const int want_frames = atoi(argv[3]);

    std::vector<unsigned char> roms(DEVICES * DEVICE_BYTES);
    FILE* rf = fopen(rom_path, "rb");
    if (!rf) { fprintf(stderr, "cannot open %s\n", rom_path); return 2; }
    size_t got = fread(roms.data(), 1, roms.size(), rf);
    fclose(rf);
    if (got != roms.size()) {
        fprintf(stderr, "%s: %zu bytes, need %zu\n", rom_path, got, roms.size());
        return 2;
    }

    std::vector<Input> inputs;
    if (argc > 4) {
        FILE* inf = fopen(argv[4], "r");
        if (!inf) { fprintf(stderr, "cannot open %s\n", argv[4]); return 2; }
        Input in;
        while (fscanf(inf, "%u %u %u", &in.sw, &in.x, &in.y) == 3) inputs.push_back(in);
        fclose(inf);
    }

    Verilated::commandArgs(argc, argv);
    VCCastles* dut = new VCCastles;

    // Master-clock cycles since the simulation began, counted here because
    // this lambda is the only thing that advances `clk` — the ROM download
    // below toggles `dn_clk`, a separate port, so it costs no master clocks.
    //
    // Frames are dated in these ticks so the harness can put the core on the
    // same timebase as our runtime and MAME. Comparing frame N against frame N
    // assumes the two sides mean the same frame, and nothing established that;
    // see harness.md section 12.
    long long master_ticks = 0;

    auto tick = [&](void) {
        dut->clk = 0; dut->eval();
        dut->clk = 1; dut->eval();
        master_ticks++;
    };

    // Hold reset, then load the ROMs through the download port, then release.
    dut->reset_n = 0;
    dut->SELFTEST = 0; dut->COCKTAIL = 0;
    dut->STARTJMP1 = 0; dut->STARTJMP2 = 0;
    dut->COINL = 0; dut->COINR = 0; dut->COINA = 0; dut->SLAM = 0;
    dut->tb1VD = 0; dut->tb1VC = 0; dut->tb1HD = 0; dut->tb1HC = 0;
    dut->dn_clk = 0; dut->dn_wr = 0; dut->dn_addr = 0; dut->dn_data = 0;
    for (int i = 0; i < 64; i++) tick();

    for (int dev = 0; dev < DEVICES; dev++) {
        for (int off = 0; off < DEVICE_BYTES; off++) {
            dut->dn_addr = (unsigned short)((dev << 13) | off);
            dut->dn_data = roms[dev * DEVICE_BYTES + off];
            dut->dn_wr = 1;
            dut->dn_clk = 0; dut->eval();
            dut->dn_clk = 1; dut->eval();
        }
    }
    dut->dn_wr = 0;
    dut->dn_clk = 0;
    for (int i = 0; i < 64; i++) tick();
    dut->reset_n = 1;

    FILE* out = fopen(out_path, "wb");
    if (!out) { fprintf(stderr, "cannot open %s\n", out_path); return 2; }

    // One master-clock count per frame written, in the same order.
    char cycles_path[512];
    snprintf(cycles_path, sizeof(cycles_path), "%s.cycles", out_path);
    FILE* cycles_out = fopen(cycles_path, "w");
    if (!cycles_out) { fprintf(stderr, "cannot open %s\n", cycles_path); return 2; }

    // The core's audio: `output [7:0] SOUT` on the top level (CCastles.v:25),
    // the sum of both POKEYs' outputs from the AudioOutput instance at :298.
    //
    // Sampled once per CPU cycle -- every eighth master clock, the same
    // division `Clock.v` makes for ce2H and the same one the .cycles sidecar
    // documents. SOUT only changes on that enable, so ANY fixed phase within
    // the eight yields the true sequence; the phase chosen here is master_ticks
    // % 8 == 0, and a comparison against it needs to allow a small constant
    // offset rather than assume a shared origin.
    //
    // Written from the first tick after reset is released, NOT from the first
    // captured frame: the sequence is what matters and its origin is
    // established by alignment, not by construction.
    char audio_path[512];
    snprintf(audio_path, sizeof(audio_path), "%s.audio", out_path);
    FILE* audio_out = fopen(audio_path, "wb");
    if (!audio_out) { fprintf(stderr, "cannot open %s\n", audio_path); return 2; }
    std::vector<unsigned char> audio;
    audio.reserve(1 << 20);

    std::vector<unsigned char> frame(WIDTH * HEIGHT * 3, 0);
    int captured = 0;
    int y = -1;            // active line within the frame
    int x = 0;             // pixel within the active line
    int emitted_max = 0;   // widest line the core actually produced
    int phase = 0;         // the core emits one pixel per two clocks
    int prev_hblank = 1, prev_vsync = 0;

    // A ceiling, so a core that never syncs cannot hang the build.
    const long long max_ticks = 700000LL * (want_frames + 4);

    // Quadrature state per axis, as a position in the Gray sequence
    // 00 -> 01 -> 11 -> 10 -> 00.
    static const int GRAY[4][2] = { {0,0}, {0,1}, {1,1}, {1,0} };
    int phase_h = 0, phase_v = 0;
    // Half-steps still owed to each axis, signed.
    int owed_h = 0, owed_v = 0;
    unsigned last_x = 0, last_y = 0;

    auto apply = [&](int f) {
        if ((size_t)f >= inputs.size()) return;
        const Input& in = inputs[f];
        dut->STARTJMP1 = (in.sw & 0x40) ? 1 : 0;
        dut->STARTJMP2 = (in.sw & 0x80) ? 1 : 0;
        dut->SELFTEST  = (in.sw & 0x10) ? 1 : 0;
        dut->SLAM      = (in.sw & 0x08) ? 1 : 0;
        dut->COINA     = (in.sw & 0x04) ? 1 : 0;
        dut->COINL     = (in.sw & 0x02) ? 1 : 0;
        dut->COINR     = (in.sw & 0x01) ? 1 : 0;

        // The harness hands us an absolute position modulo 256, the same figure
        // MAME's LETA fields are given. Turn the change since last frame into
        // quadrature half-steps: SupportChips.v:297-307 exposes
        // count = counter[8:1], so two half-steps make one CPU-visible count.
        int dx = (int)in.x - (int)last_x;
        if (dx > 128) dx -= 256; else if (dx < -128) dx += 256;
        int dy = (int)in.y - (int)last_y;
        if (dy > 128) dy -= 256; else if (dy < -128) dy += 256;
        last_x = in.x;
        last_y = in.y;
        owed_h += dx * 2;
        owed_v += dy * 2;
    };
    apply(0);

    // Emit at most one half-step per axis per pixel clock, so a frame's worth of
    // movement is spread across the frame rather than delivered as a burst.
    auto step_quadrature = [&](void) {
        if (owed_h > 0)      { phase_h = (phase_h + 1) & 3; owed_h--; }
        else if (owed_h < 0) { phase_h = (phase_h + 3) & 3; owed_h++; }
        if (owed_v > 0)      { phase_v = (phase_v + 1) & 3; owed_v--; }
        else if (owed_v < 0) { phase_v = (phase_v + 3) & 3; owed_v++; }
        // CCastles.v:292 wires .X2(tb1HD), .Y2(tb1HC) as the horizontal pair and
        // .X1(tb1VD), .Y1(tb1VC) as the vertical. Which of each pair leads is
        // not established, so a reversed axis would show as inverted motion --
        // recorded, not asserted.
        dut->tb1HD = GRAY[phase_h][0];
        dut->tb1HC = GRAY[phase_h][1];
        dut->tb1VD = GRAY[phase_v][0];
        dut->tb1VC = GRAY[phase_v][1];
    };

    for (long long t = 0; t < max_ticks && captured < want_frames; t++) {
        tick();

        // One sample per CPU cycle, at a fixed phase within the eight.
        if (master_ticks % 8 == 0) {
            audio.push_back((unsigned char)dut->SOUT);
        }

        int vsync = dut->VSYNC;
        int hblank = dut->HBLANK;
        int vblank = dut->VBLANK;

        // Frame boundary on the rising edge of VSYNC.
        if (vsync && !prev_vsync) {
            if (y >= 0) {
                fwrite(frame.data(), 1, frame.size(), out);
                // Dated at the same instant the pixels are written, so stamp k
                // and frame k are one moment rather than one frame number.
                fprintf(cycles_out, "%lld\n", master_ticks);
                captured++;
                apply(captured);
            }
            std::fill(frame.begin(), frame.end(), 0);
            y = -1;
        }
        prev_vsync = vsync;

        // Falling edge of HBLANK starts an active line.
        if (!hblank && prev_hblank) {
            if (!vblank) y++;
            x = 0;
            phase = 0;
        }
        prev_hblank = hblank;

        if (!hblank && !vblank && y >= 0 && y < HEIGHT) {
            if (phase == 0 && x < WIDTH) {
                unsigned rgb = dut->RGBout;
                unsigned char* p = &frame[(y * WIDTH + x) * 3];
                p[0] = scale3((rgb >> 6) & 7);   // red
                p[1] = scale3((rgb >> 3) & 7);   // green
                p[2] = scale3(rgb & 7);          // blue
                x++;
                if (x > emitted_max) emitted_max = x;
            }
            phase ^= 1;
            if (phase == 0) step_quadrature();
        }
    }

    fclose(out);
    fclose(cycles_out);
    fwrite(audio.data(), 1, audio.size(), audio_out);
    fclose(audio_out);

    char dims[512];
    snprintf(dims, sizeof(dims), "%s.dims", out_path);
    FILE* df = fopen(dims, "w");
    if (df) {
        fprintf(df, "%d %d %d\n", WIDTH, HEIGHT, emitted_max);
        fclose(df);
    }

    fprintf(stderr, "mistersim: %d frames, widest active line %d pixels, %zu audio samples\n",
            captured, emitted_max, audio.size());
    delete dut;
    return captured == want_frames ? 0 : 1;
}
