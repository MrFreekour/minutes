import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "whisper.cpp vs parakeet.cpp for local transcription — minutes",
  description:
    "A candid comparison of two local transcription engines: accuracy, Apple Silicon speed, language coverage, build friction, and why Minutes currently selects sealed Whisper.",
  alternates: {
    canonical: "/writing/whisper-cpp-vs-parakeet-cpp",
  },
};

const benchmarks = [
  { engine: "Whisper", model: "small (our default)", params: "244M", wer: "3.4%", speed: "~200ms" },
  { engine: "Whisper", model: "medium", params: "769M", wer: "2.9%", speed: "~600ms" },
  { engine: "Whisper", model: "large-v3", params: "1.55B", wer: "2.4%", speed: "~1.5s" },
  { engine: "Parakeet", model: "tdt-ctc-110m", params: "110M", wer: "2.4%", speed: "~27ms" },
  { engine: "Parakeet", model: "tdt-600m", params: "600M", wer: "1.7%", speed: "~520ms" },
] as const;

export default function Post() {
  return (
    <div className="mx-auto max-w-[680px] px-6 pb-16 sm:px-8">
      <nav className="flex items-center justify-between border-b border-[color:var(--border)] py-4">
        <a href="/" className="font-mono text-[15px] font-medium text-[var(--text)]">
          minutes
        </a>
        <div className="flex gap-x-6 text-sm text-[var(--text-secondary)]">
          <a href="/writing" className="hover:text-[var(--accent)]">
            Writing
          </a>
          <a
            href="https://github.com/silverstein/minutes"
            className="hover:text-[var(--accent)]"
          >
            GitHub
          </a>
        </div>
      </nav>

      <article className="pt-14">
        <p className="mb-4 font-mono text-[11px] uppercase tracking-[0.18em] text-[var(--text-secondary)]">
          2026-07-11 · Mat Silverstein
        </p>
        <h1 className="font-serif text-[36px] leading-[1.05] tracking-[-0.04em] text-[var(--text)] sm:text-[42px]">
          whisper.cpp vs parakeet.cpp for local transcription
        </h1>

        <div className="mt-8 space-y-5 text-[16px] leading-[1.75] text-[var(--text-secondary)]">
          <p>
            Minutes retains integrations for both engines, but currently selects only its sealed,
            in-process whisper.cpp path. The available parakeet.cpp helpers accept audio by
            pathname, which cannot safely receive Minutes&apos; sealed private-audio capability.
            Parakeet therefore fails closed on every platform until it has a safe byte transport.
            The upstream performance comparison is still useful, so here are the numbers and the
            friction we found while building the integration.
          </p>

          <h2 className="pt-4 font-serif text-[26px] leading-tight tracking-[-0.02em] text-[var(--text)]">
            The numbers
          </h2>
          <p>
            Parakeet is NVIDIA&apos;s FastConformer architecture;{" "}
            <a
              href="https://github.com/Frikallo/parakeet.cpp"
              className="text-[var(--text)] underline decoration-[color:var(--border-mid)] underline-offset-2 hover:text-[var(--accent)]"
            >
              parakeet.cpp
            </a>{" "}
            is its ggml-style local port with Metal acceleration on Apple Silicon. LibriSpeech
            clean word-error rates, with speed measured on 10 seconds of audio on an M-series GPU:
          </p>

          <div className="overflow-x-auto rounded-[8px] border border-[color:var(--border)] bg-[var(--bg-elevated)]">
            <table className="min-w-full border-collapse text-left">
              <thead>
                <tr className="border-b border-[color:var(--border)]">
                  {["Engine", "Model", "Params", "WER", "Speed"].map((h) => (
                    <th
                      key={h}
                      className="px-4 py-3 font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--text-secondary)]"
                    >
                      {h}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {benchmarks.map((row) => (
                  <tr
                    key={`${row.engine}-${row.model}`}
                    className="border-b border-[color:var(--border)] last:border-b-0"
                  >
                    <td className="px-4 py-3 font-mono text-[13px] text-[var(--text)]">{row.engine}</td>
                    <td className="px-4 py-3 font-mono text-[13px] text-[var(--text-secondary)]">{row.model}</td>
                    <td className="px-4 py-3 font-mono text-[13px] text-[var(--text-secondary)]">{row.params}</td>
                    <td className="px-4 py-3 font-mono text-[13px] text-[var(--text-secondary)]">{row.wer}</td>
                    <td className="px-4 py-3 font-mono text-[13px] text-[var(--text-secondary)]">{row.speed}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          <p>
            Read that table twice. Parakeet&apos;s 110M-parameter model matches Whisper
            large-v3&apos;s accuracy with 14× fewer parameters, and transcribes 10 seconds of
            audio in 27 milliseconds where large-v3 takes a second and a half. The 600M model
            beats everything in its class at 1.7% WER. On raw accuracy-per-parameter and speed,
            it isn&apos;t close.
          </p>

          <h2 className="pt-4 font-serif text-[26px] leading-tight tracking-[-0.02em] text-[var(--text)]">
            So why is Whisper still our default?
          </h2>
          <p>
            <span className="font-medium text-[var(--text)]">Languages.</span> Whisper covers 99
            languages. Parakeet&apos;s tdt-600m covers 25 European ones, and the 110M model is
            English-only. If your meetings might contain Japanese, Hindi, Arabic, or Mandarin,
            the comparison is over before it starts.
          </p>
          <p>
            <span className="font-medium text-[var(--text)]">Zero-friction install.</span>{" "}
            whisper.cpp has mature prebuilt distribution everywhere. parakeet.cpp has no binary
            releases as of this writing: you build it yourself with CMake — and on macOS you need
            full Xcode for the Metal shader compiler, plus CMake 3.31.x specifically, because a
            bundled dependency trips on CMake 4. Then you download a 2.4&nbsp;GB .nemo file from
            HuggingFace and convert it to safetensors with a small Python venv. We documented the
            whole path and it&apos;s reliable — but it&apos;s an afternoon, not a{" "}
            <span className="font-mono text-[14px]">brew install</span>.
          </p>
          <p>
            <span className="font-medium text-[var(--text)]">Streaming partials.</span> Our
            dictation overlay depends on fast mid-utterance partial results, and Whisper&apos;s
            streaming behavior is what makes that feel live. The retained Parakeet integration
            does not currently receive production audio because its pathname-only handoff would
            require restoring plaintext staging.
          </p>
          <p>
            <span className="font-medium text-[var(--text)]">Private-audio transport.</span>{" "}
            A transcription engine must consume audio without creating a named plaintext file
            that another same-user process can open. Whisper runs in-process against Minutes&apos;
            sealed reader. The current Parakeet subprocess and warm-server protocols accept a
            pathname, so Minutes rejects Parakeet selection and uses Whisper instead.
          </p>

          <h2 className="pt-4 font-serif text-[26px] leading-tight tracking-[-0.02em] text-[var(--text)]">
            Where Parakeet&apos;s upstream results stand out
          </h2>
          <p>
            English or major-European-language audio on Apple Silicon, especially live
            transcription. The latency difference is not subtle: for real-time meeting
            transcription, a warm Parakeet sidecar turns per-utterance transcription from
            &quot;noticeable lag&quot; into &quot;effectively instant,&quot; and on long batch
            jobs the throughput gap compounds. One contributor runs Parakeet through
            NVIDIA&apos;s NeMo on an RTX 3090: a 68-minute French meeting transcribes in about
            3.5 minutes, with quality that beats Whisper large-v3 on mixed-language audio.
          </p>

          <h2 className="pt-4 font-serif text-[26px] leading-tight tracking-[-0.02em] text-[var(--text)]">
            The recommendation
          </h2>
          <p>
            Use Whisper in Minutes today. Even if a Parakeet preference or binary is present,
            Minutes resolves to sealed Whisper rather than stage private audio on a pathname.
            Parakeet selection can return when its subprocess accepts a safe byte stream, or when
            an explicitly acknowledged descriptor-isolation protocol closes the same-user open
            race. Until then, the benchmark advantage is promising research, not a setup path we
            recommend to Minutes users.
          </p>
          <p>
            The retained integration notes, including the safety gate and build pitfalls, are in{" "}
            <a
              href="https://github.com/silverstein/minutes/blob/main/docs/architecture/parakeet.md"
              className="text-[var(--text)] underline decoration-[color:var(--border-mid)] underline-offset-2 hover:text-[var(--accent)]"
            >
              docs/architecture/parakeet.md
            </a>
            . Benchmarks are the upstream projects&apos; published LibriSpeech numbers; speed
            figures are from parakeet.cpp&apos;s measurements on M-series GPUs. Minutes is MIT
            licensed —{" "}
            <a
              href="https://github.com/silverstein/minutes"
              className="text-[var(--text)] underline decoration-[color:var(--border-mid)] underline-offset-2 hover:text-[var(--accent)]"
            >
              the pipeline code is on GitHub
            </a>
            .
          </p>
        </div>
      </article>
    </div>
  );
}
