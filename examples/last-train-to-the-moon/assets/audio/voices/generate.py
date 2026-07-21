#!/usr/bin/env python3
"""Generate editable character voice clips without storing credentials."""

from __future__ import annotations

import json
import os
from pathlib import Path

from dotenv import load_dotenv
from elevenlabs import VoiceSettings
from elevenlabs.client import ElevenLabs


ROOT = Path(__file__).resolve().parent


def main() -> None:
    env_path = Path.home() / ".nanobanana.env"
    load_dotenv(env_path, override=True)
    api_key = os.getenv("ELEVENLABS_API_KEY")
    if not api_key and env_path.exists():
        for line in env_path.read_text(encoding="utf-8").splitlines():
            if line.startswith("ELEVENLABS_API_KEY="):
                api_key = line.split("=", 1)[1].strip()
                break
    if not api_key:
        raise SystemExit("ELEVENLABS_API_KEY is not configured")

    lines = json.loads((ROOT / "lines.json").read_text(encoding="utf-8"))
    client = ElevenLabs(api_key=api_key)
    settings = VoiceSettings(
        stability=0.42,
        similarity_boost=0.8,
        style=0.38,
        use_speaker_boost=True,
    )

    for index, line in enumerate(lines):
        output = ROOT / f"{line['id']}.mp3"
        if output.exists():
            print(f"{output.name} (existing)")
            continue
        audio = client.text_to_speech.convert(
            voice_id=line["voiceId"],
            text=line["text"],
            model_id="eleven_multilingual_v2",
            output_format="mp3_44100_128",
            voice_settings=settings,
            seed=5610 + index,
        )
        with output.open("wb") as stream:
            for chunk in audio:
                stream.write(chunk)
        print(output.name)


if __name__ == "__main__":
    main()
