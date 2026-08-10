-- Drive the machine from a recorded trace and capture every frame.
--
-- Used by chill65-diff through MAME's -autoboot_script. Parameters arrive as
-- environment variables because -autoboot_script takes no arguments of its own.
--
--   CHILL65_MAME_OUT     file to write raw frames to
--   CHILL65_MAME_FRAMES  how many frames to capture before exiting
--   CHILL65_MAME_INPUT   optional; one line per frame, "<IN0 mask> <x> <y>"
--
-- Writes a sidecar "<out>.dims" holding "<width> <height>", so the Rust side
-- can check the geometry it was given rather than assuming it.
--
-- Writes a second sidecar "<out>.cycles", one decimal per captured frame: the
-- emulated CPU cycle count at the instant that frame was taken. The harness
-- compares frame N against frame N, which assumes the two sides mean the same
-- frame; nothing established that (harness.md §12), so every frame is dated
-- and the offset becomes a measurement.
--
-- MAME 0.276's Lua has no cycle counter on the CPU device -- `total_cycles()`,
-- `cycles_remaining()`, `.clock` and `.clockscale` are all absent. What it does
-- have is machine time as an attotime, and `as_ticks` converts that to any
-- frequency in exact integer arithmetic. At the 1.25 MHz CPU clock the count
-- rises by exactly 20480 a frame, which is `CYCLES_PER_FRAME`. Integers, so it
-- cannot drift over a long trace the way `as_double()` would.

local out_path = assert(os.getenv("CHILL65_MAME_OUT"), "CHILL65_MAME_OUT unset")

-- Split rather than nested: assert returns *all* its arguments, so
-- tonumber(assert(v, msg)) passes the message along as tonumber's base.
local frames_env = assert(os.getenv("CHILL65_MAME_FRAMES"), "CHILL65_MAME_FRAMES unset")
local want = assert(tonumber(frames_env), "CHILL65_MAME_FRAMES is not a number")

local screen = manager.machine.screens[":screen"]
assert(screen, "the machine has no :screen")

local dims = assert(io.open(out_path .. ".dims", "w"))
dims:write(string.format("%d %d\n", screen.width, screen.height))
dims:close()

-- Inputs, one entry per frame: { IN0 mask, trackball X, trackball Y }.
local inputs = {}
local input_path = os.getenv("CHILL65_MAME_INPUT")
if input_path then
    for line in io.lines(input_path) do
        local sw, x, y = line:match("^(%d+)%s+(%d+)%s+(%d+)")
        if sw then
            inputs[#inputs + 1] = { tonumber(sw), tonumber(x), tonumber(y) }
        end
    end
end

-- The switch fields of :IN0, one per mask. Several fields share a mask -- the
-- upright and cocktail namings of the same physical input -- so keying by mask
-- rather than by name avoids picking arbitrarily between them.
local ports = manager.machine.ioport.ports
local switches = {}
for _, field in pairs(ports[":IN0"].fields) do
    switches[field.mask] = switches[field.mask] or field
end

-- LETA0 is vertical and LETA1 horizontal (CCastles.v:292, and the game's own
-- HW.TBV/HW.TBH equates at CG.MAC:120-121).
local trackball_y = ports[":LETA0"].fields["Trackball Y"]
local trackball_x = ports[":LETA1"].fields["Trackball X"]

-- Integer bit test without bitwise operators, so this works whichever Lua MAME
-- was built against. Every mask here is a single bit.
local function pressed(value, mask)
    return math.floor(value / mask) % 2 == 1
end

local function apply(frame)
    local entry = inputs[frame + 1]
    if not entry then
        return
    end
    for mask, field in pairs(switches) do
        field:set_value(pressed(entry[1], mask) and 1 or 0)
    end
    -- The LETA reports absolute position, not a delta, so the harness hands us
    -- an accumulated count and we set it directly.
    trackball_x:set_value(entry[2])
    trackball_y:set_value(entry[3])
end

-- The CPU clock, and so the unit every cycle stamp is expressed in.
-- 1,250,000 Hz over 61.035 frames a second is 20,480 cycles a frame; see the
-- clock chain in crates/chill65-runtime/src/frame.rs.
local CPU_HZ = 1250000

local out = assert(io.open(out_path, "wb"))
local cycles_out = assert(io.open(out_path .. ".cycles", "w"))
local seen = 0

apply(0)

-- The subscription must be kept alive: dropping it unsubscribes.
notifier = emu.add_machine_frame_notifier(function()
    if seen >= want then
        return
    end
    -- Parenthesised deliberately: pixels() returns (data, width, height), and
    -- write() would happily append "256232" to every frame.
    out:write((screen:pixels()))
    -- Dated in the same callback as the pixels, so stamp k and frame k are the
    -- same instant rather than merely the same frame number.
    cycles_out:write(string.format("%d\n", manager.machine.time:as_ticks(CPU_HZ)))
    seen = seen + 1
    if seen >= want then
        out:close()
        cycles_out:close()
        manager.machine:exit()
    else
        apply(seen)
    end
end)
