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

    auto tick = [&](void) {
        dut->clk = 0; dut->eval();
        dut->clk = 1; dut->eval();
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

    std::vector<unsigned char> frame(WIDTH * HEIGHT * 3, 0);
    int captured = 0;
    int y = -1;            // active line within the frame
    int x = 0;             // pixel within the active line
    int emitted_max = 0;   // widest line the core actually produced
    int phase = 0;         // the core emits one pixel per two clocks
    int prev_hblank = 1, prev_vsync = 0;

    // A ceiling, so a core that never syncs cannot hang the build.
    const long long max_ticks = 700000LL * (want_frames + 4);

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
        // Trackball quadrature is task #14's problem; the pins stay put here.
    };
    apply(0);

    for (long long t = 0; t < max_ticks && captured < want_frames; t++) {
        tick();

        int vsync = dut->VSYNC;
        int hblank = dut->HBLANK;
        int vblank = dut->VBLANK;

        // Frame boundary on the rising edge of VSYNC.
        if (vsync && !prev_vsync) {
            if (y >= 0) {
                fwrite(frame.data(), 1, frame.size(), out);
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
        }
    }

    fclose(out);

    char dims[512];
    snprintf(dims, sizeof(dims), "%s.dims", out_path);
    FILE* df = fopen(dims, "w");
    if (df) {
        fprintf(df, "%d %d %d\n", WIDTH, HEIGHT, emitted_max);
        fclose(df);
    }

    fprintf(stderr, "mistersim: %d frames, widest active line %d pixels\n",
            captured, emitted_max);
    delete dut;
    return captured == want_frames ? 0 : 1;
}
