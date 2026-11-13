# Bass Booster

A small Rust program that boosts the bass of whatever is playing on your PC (songs, videos, etc.). You pick a value from 0 to 100 and can change it live while the audio is playing.

It works by capturing audio from a virtual cable, shaping the low end with an EQ and sending the result to your real speakers or headphones.

## How the sound is shaped

- A 30 Hz high-pass filter removes sub-bass rumble that adds no punch and eats headroom.
- A main boost band around 80 Hz (kick drums and bass fundamentals).
- A smaller boost band around 140 Hz (gives definition to bass lines, so it sounds like a bass and not just a rumble).
- The overall level is lowered a bit as the boost goes up, so the bass doesn't clip or distort. Raise your headphone volume if it feels quieter than before.

### Thump mode

Optional mode (`--thump` flag, or press `t` while it is running) for a deeper, tremble-in-your-ears feel:

- A sub-bass boost around 50 Hz.
- A compressor on the low end that lifts the tail of each kick, so the hit feels longer and heavier.
- Generated harmonics of the bass (roughly 100-300 Hz). Your brain reads them as deeper bass even if the headphones can't reproduce the real sub-bass, which matters a lot for in-ear headphones.

## Requirements

- Windows 11
- Rust and cargo
- [VB-Cable](https://vb-audio.com/Cable/) (virtual audio device)

## One-time setup

1. Install VB-Cable (run the installer as administrator) and restart the PC.
2. Open **Settings → System → Sound** and set **CABLE Input** as the default output device. Do not pick *CABLE In 16ch*.
3. Make sure both devices use the same audio format, otherwise the streams may fail to open:
   - Open **More sound settings**.
   - **Playback** tab → your speakers → **Properties → Advanced**.
   - **Recording** tab → **CABLE Output** → **Properties → Advanced**.
   - Set both to **24 bit, 48000 Hz**.

## Usage

List the audio devices to get their exact names:

```
cargo run --release -- --list
```

Run the program (adjust the output name to match your speakers or headphones):

```
cargo run --release -- 60 --input "CABLE Output" --output "Altavoces"
```

| Argument | Description |
|---|---|
| `AMOUNT` | Bass boost from 0 to 100 (default: 50) |
| `--input NAME` | Input device name, or part of it (case-insensitive) |
| `--output NAME` | Output device name, or part of it (case-insensitive) |
| `--thump` | Start with thump mode enabled |
| `--list` | List available audio devices |
| `--help` | Show usage |

While it is running:

- Type a number from `0` to `100` and press Enter to change the bass instantly.
- Type `t` and press Enter to toggle thump mode on/off.
- Type `q` and press Enter to quit.

## When you are done

Set your normal speakers or headphones back as the default output device in Windows Sound settings. If you leave **CABLE Input** selected, you will hear nothing.

## Tweaking

All the values are constants at the top of `src/main.rs`:

| Constant | Default | What it does |
|---|---|---|
| `MAX_DB` | `12.0` | Boost applied at value 100. Lower it if it still feels too aggressive. |
| `PUNCH_HZ` | `80.0` | Main bass band. Try 60-70 for more thump, 90-100 for more punch. |
| `PUNCH_Q` | `0.8` | Width of the main band. Higher = narrower. |
| `BODY_HZ` | `140.0` | Upper bass band. |
| `BODY_WEIGHT` | `0.5` | Gain of the body band relative to the main band. Raise it for more bass-line definition. |
| `HEADROOM` | `0.5` | How much of the boost is compensated by lowering the overall level. Higher = safer against distortion but quieter. |
| `SUB_HZ` | `50.0` | Sub-bass band used in thump mode. |
| `SUB_WEIGHT` | `0.8` | Sub-bass gain relative to the main band (thump mode). |
| `HARM_MIX` | `0.35` | Amount of generated harmonics (thump mode). Raise it for a deeper feel on small headphones. |
| `COMP_RELEASE_MS` | `250.0` | How long the kick tail is held up (thump mode). Higher = longer thump. |
| `THUMP_MAKEUP_DB` | `4.0` | Extra level the compressor gives back to the low end (thump mode). |

If you hear crackling or dropouts, increase `max_samples` in `main`. If there is too much delay, decrease it.

## FAQ

**I run it but I don't hear any difference.**
Windows is not sending the audio through the virtual cable, so your music goes straight to the headphones. Fix:
1. In **Settings → System → Sound → Output**, select **CABLE Input (VB-Audio Virtual Cable)**.
2. In **Volume mixer**, make sure your player's output device is *Default* or *CABLE Input*, not your real speakers.
3. Close and reopen the player (many apps keep using the old device until restarted).
4. Quick test: with music playing and the program running, type `q`. If the music keeps playing the same, it is not going through the program. If it stops, it is.

**I only see the microphone as an input, no CABLE.**
VB-Cable is not installed yet, or the PC was not restarted after installing it. Run `--list` again after restarting.

**I used my microphone as the input and now I hear myself.**
The input must be `CABLE Output`, not the microphone. Close the program with `q`.

**I hear nothing at all.**
Either the program is not running, or the default output is still *CABLE Input*. Start the program, or set your headphones back as the default output.

**It says the device was not found.**
Run `--list` and use a part of the exact name shown there.

**Error opening the stream.**
The sample rate or channels of CABLE Output and your speakers don't match. Redo step 3 of the setup (24 bit, 48000 Hz on both).

**The bass sounds distorted or harsh ("earrape").**
Use a lower value (30-50), lower `MAX_DB`, or raise `HEADROOM`. Also check the volume of your player and of Windows, since a very loud source clips more easily.

**The music is quieter now.**
It is intentional: the overall level is lowered a bit to leave room for the bass. Raise your headphone volume.

**Thump mode doesn't make my ears tremble.**
That feeling is real sub-bass energy, so it also depends on the headphones: driver size, a good seal in the ear and how low they can actually go. Thump mode helps a lot with generated harmonics, but it can't make small drivers move more than they physically can. Try raising `HARM_MIX` and `SUB_WEIGHT` a little, and check that the ear tips seal well.

**Thump mode sounds distorted.**
Use a lower value (30-50), or lower `HARM_MIX`, `SUB_WEIGHT` and `THUMP_MAKEUP_DB`. Strong sub-bass is also tiring and risky for hearing without feeling loud, so keep the volume moderate.

**Choppy audio.**
Always compile with `--release`.
