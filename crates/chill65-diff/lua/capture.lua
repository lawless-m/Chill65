-- Capture every emulated frame's pixels to a file, then quit.
--
-- Driven by chill65-diff through MAME's -autoboot_script. Parameters arrive as
-- environment variables because -autoboot_script takes no arguments of its own.
--
--   CHILL65_MAME_OUT     file to append raw frames to
--   CHILL65_MAME_FRAMES  how many frames to capture before exiting
--
-- Writes a sidecar "<out>.dims" holding "<width> <height>", so the Rust side
-- can check the geometry it was given rather than assuming it.

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

local out = assert(io.open(out_path, "wb"))
local seen = 0

-- The subscription must be kept alive: dropping it unsubscribes.
notifier = emu.add_machine_frame_notifier(function()
    if seen >= want then
        return
    end
    -- Parenthesised deliberately: pixels() returns (data, width, height), and
    -- write() would happily append "256232" to every frame.
    out:write((screen:pixels()))
    seen = seen + 1
    if seen >= want then
        out:close()
        manager.machine:exit()
    end
end)
