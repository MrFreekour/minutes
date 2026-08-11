#!/usr/bin/env node

import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const chrome =
  process.env.ARCHIVE_TEST_CHROME ??
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const profile = await mkdtemp(join(tmpdir(), "minutes-archive-ui-"));
const screenshot = join(profile, "search-proof.png");
const updateScreenshot = join(profile, "update-proof.png");
const frontend = pathToFileURL(resolve("archive/src/index.html")).href;

const census = {
  schema: "minutes.archive-census.v1",
  status: "complete",
  privacy: {
    document_content_read: false,
    filenames_emitted: false,
    paths_emitted: false,
    symlinks_followed: false,
    hashes_computed: false,
  },
  summary: {
    approved_locations: 1,
    artifacts: 4,
    regular_files: 4,
    packages: 0,
    regular_file_bytes: 4096,
    directories_scanned: 1,
  },
  formats: [
    {
      extension: ".txt",
      category: "plain_text",
      files: 3,
      packages: 0,
      regular_file_bytes: 3072,
    },
    {
      extension: ".pdf",
      category: "pdf",
      files: 1,
      packages: 0,
      regular_file_bytes: 1024,
    },
  ],
  categories: [
    { category: "plain_text", artifacts: 3, regular_file_bytes: 3072 },
    { category: "pdf", artifacts: 1, regular_file_bytes: 1024 },
    // Enough scans that the forecast crosses its one-minute threshold, so the
    // assertion below tests the estimate rather than the hidden state.
    { category: "image_or_scan", artifacts: 400, regular_file_bytes: 40960 },
  ],
  age_buckets: {},
  size_buckets: {},
  signals: {
    symlinks_skipped: 0,
    hidden_artifacts: 0,
    icloud_placeholders: 0,
    zero_byte_files: 0,
    permission_mode_unreadable: 0,
    special_files_skipped: 0,
    metadata_errors: 0,
    directory_errors: 0,
    max_depth: 1,
  },
};

const vaultReport = {
  schema: "minutes.archive-document-vault.v1",
  vault_id: "local-private-vault",
  approved_locations: 1,
  indexed_documents: 3,
  inferred_boundary_documents: 0,
  indexed_bytes: 3072,
  unsupported_files_skipped: 1,
  oversized_files_skipped: 0,
  malformed_text_files_skipped: 0,
  conversion_failures: 0,
  ocr_required_files: 0,
  searchable_pdf_documents: 1,
  docx_documents: 1,
  duplicate_files_skipped: 0,
  transcribed_documents: 4,
  mixed_provenance_documents: 2,
  budget_reached: true,
  documents_left_unread: 5000,
  directories_left_unread: 3,
  symlinks_skipped: 2,
  hard_links_skipped: 3,
  permission_denied: 9000,
  unopenable: 4,
  scans_unreadable: 12,
  entries_unstattable: 0,
  identity_unavailable: 0,
  changed_while_reading: 2,
  directory_errors: 0,
  source_content_persisted: false,
  retrieval_index_persisted: false,
  converter_sandbox_verified: true,
  semantic_worker_sandbox_verified: true,
  excluded_directories: 2,
  excluded_folder_changes: 1,
  semantic_retrieval_enabled: true,
  // The worker died partway through this build. Partial suggestion coverage
  // must be stated, not inferred from a smaller vector count.
  semantic_coverage_partial: true,
  semantic_model: {
    model_id: "apple-nl-sentence-en-r1",
    revision: 1,
    dimension: 512,
    built_in_os_asset: true,
    model_download_requested: false,
  },
  semantic_provisions_indexed: 4,
  semantic_provisions_skipped: 7,
  semantic_derivatives_persisted: false,
  semantic_model_download_requested: false,
  supported_formats: [".docx", ".md", ".pdf", ".text", ".txt"],
};

const evidence = {
  query: {
    raw: "Find confidentiality provisions under three sentences covering affiliates.",
    scope: "same_provision",
    required_concepts: ["confidentiality", "affiliates"],
    excluded_concepts: [],
    exact_phrase: null,
    max_sentences: 3,
    limit: 20,
  },
  evidence: [
    {
      vault_id: "local-private-vault",
      document_id: "document-0000000000000001",
      document_title: "Synthetic Agreement",
      provision_heading: "7. CONFIDENTIALITY",
      source_anchor: "section:0001",
      exact_excerpt:
        "Confidential Information includes information of Recipient and its affiliates.",
      sentence_count: 1,
      source_revision: { sha256: "00", byte_len: 76 },
      source_converter: "pdf-extract-0.12.0-v1",
      matched_concepts: ["confidentiality", "affiliates"],
      why_matched:
        "Matched confidentiality, affiliates, sentence limit in the same provision; 1 sentence.",
      lexical_rank: -2.1,
      index_fresh: true,
    },
  ],
  documents: [
    {
      document_title: "Capped Evidence Agreement",
      matched_concepts: ["confidentiality", "assignment"],
      criterion_evidence: [
        {
          document_id: "document-0000000000000004",
          document_title: "Capped Evidence Agreement",
          provision_heading: "65. ASSIGNMENT",
          source_anchor: "section:0065",
          exact_excerpt: "Neither party may assign this Agreement without consent.",
          sentence_count: 1,
          source_converter: "utf8-text-v1",
          why_matched: "Matched assignment in the same provision; 1 sentence.",
          index_fresh: true,
        },
      ],
      criterion_evidence_truncated: true,
      why_matched: "Matched confidentiality, assignment across 64 provisions in this document.",
      index_fresh: true,
    },
  ],
  semantic_suggestions: [
    {
      vault_id: "local-private-vault",
      document_id: "document-0000000000000002",
      document_title: "Meaning Similar Agreement",
      provision_heading: "8. NONDISCLOSURE",
      source_anchor: "paragraph:000021/section:0001",
      exact_excerpt:
        "The recipient must protect all nonpublic business material from disclosure.",
      sentence_count: 1,
      source_revision: { sha256: "11", byte_len: 82 },
      source_converter: "docx-xml-0.41.0-v1",
      semantic_similarity: 0.31,
      why_suggested:
        "Meaning-similar suggestion from a revision-pinned on-device model; review the exact excerpt. This is not a determination of legal sufficiency.",
      index_fresh: true,
    },
  ],
  transcriptions: [
    {
      vault_id: "local-private-vault",
      document_id: "document-0000000000000003",
      document_title: "Scanned Exhibit C",
      page_anchor: "page:0004",
      transcribed_text:
        "CONFIDENTIALITY. The Recipient shall protect Confidential Information.",
      lowest_line_confidence: 0.62,
      source_revision: { sha256: "22", byte_len: 91 },
      transcriber: "apple-vision-text-r3",
      matched_concepts: ["confidentiality"],
      why_transcribed:
        "Read from a scanned image. These characters are a machine's reading of the page, not the document's own text; check them against the source before relying on them.",
      index_fresh: true,
    },
  ],
  lexical_candidates_considered: 1,
  semantic_candidates_considered: 4,
  semantic_query_applied: true,
  semantic_model: {
    model_id: "apple-nl-sentence-en-r1",
    revision: 1,
    dimension: 512,
    built_in_os_asset: true,
    model_download_requested: false,
  },
  stale_evidence_withdrawn: 0,
  inferred_boundary_evidence_withdrawn: 0,
};

const mockScript = `
  window.__TAURI__ = {
    core: {
      invoke: async (command) => {
        const responses = ${JSON.stringify({
          archive_bootstrap: {
            buildIdentity: "v0.24.0 · build 39773037b5d5",
            locations: [],
            scanRunning: false,
            report: null,
            textVaultReport: null,
            update: { state: "notChecked" },
          },
          choose_archive_locations: {
            locations: [{ id: 1, label: "Approved location 1" }],
            // One of the chosen folders was already inside an approved one.
            // The interface must show the location it kept, not refuse the
            // batch and clear the list.
            folded: 1,
            // The owner chose a folder they had previously skipped.
            unskipped: 1,
            // The folded location had a skipped folder inside it.
            forgottenSkips: 1,
          },
          choose_archive_exclusions: {
            skipped: 1,
            outside: 1,
            refusedWholeLocation: 0,
            total: 3,
          },
          clear_archive_exclusions: 0,
          // The one network surface, exercised the way an operator meets it:
          // an offer at launch that must be visible, must name both versions,
          // and must not download anything until the button is pressed.
          check_for_archive_update: {
            state: "available",
            installed: "0.1.0",
            offered: "0.2.0",
          },
          install_archive_update: { state: "installed", offered: "0.2.0" },
          remove_archive_location: [],
          run_archive_census: census,
          cancel_archive_census: true,
          export_archive_census: false,
          build_archive_text_vault: vaultReport,
          search_archive_text_vault: evidence,
        })};
        return structuredClone(responses[command]);
      }
    }
  };
`;

class Cdp {
  constructor(url) {
    this.socket = new WebSocket(url);
    this.nextId = 1;
    this.pending = new Map();
    this.socket.onmessage = (event) => {
      const message = JSON.parse(event.data);
      if (!message.id) return;
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      if (message.error) pending.reject(new Error(message.error.message));
      else pending.resolve(message.result);
    };
  }

  async ready() {
    if (this.socket.readyState === WebSocket.OPEN) return;
    await new Promise((resolveOpen, rejectOpen) => {
      this.socket.onopen = resolveOpen;
      this.socket.onerror = rejectOpen;
    });
  }

  send(method, params = {}) {
    const id = this.nextId++;
    return new Promise((resolveResult, rejectResult) => {
      this.pending.set(id, { resolve: resolveResult, reject: rejectResult });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  close() {
    this.socket.close();
  }
}

const browser = spawn(
  chrome,
  [
    "--headless=new",
    "--disable-gpu",
    "--hide-scrollbars",
    "--allow-file-access-from-files",
    "--remote-debugging-port=0",
    `--user-data-dir=${profile}`,
    "--window-size=1080,900",
    "about:blank",
  ],
  { stdio: "ignore" },
);

async function waitForFile(path, attempts = 100) {
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    try {
      return await readFile(path, "utf8");
    } catch {
      await new Promise((resolveWait) => setTimeout(resolveWait, 50));
    }
  }
  throw new Error(`Timed out waiting for ${path}`);
}

try {
  const [port] = (await waitForFile(join(profile, "DevToolsActivePort"))).split("\n");
  const pages = await fetch(`http://127.0.0.1:${port}/json/list`).then((response) =>
    response.json(),
  );
  const page = pages.find((candidate) => candidate.type === "page");
  if (!page) throw new Error("Chrome did not expose a test page");
  const cdp = new Cdp(page.webSocketDebuggerUrl);
  await cdp.ready();
  await cdp.send("Page.enable");
  await cdp.send("Runtime.enable");
  await cdp.send("Page.addScriptToEvaluateOnNewDocument", { source: mockScript });
  await cdp.send("Page.navigate", { url: frontend });
  await new Promise((resolveWait) => setTimeout(resolveWait, 250));

  const smoke = await cdp.send("Runtime.evaluate", {
    awaitPromise: true,
    returnByValue: true,
    expression: `
      (async () => {
        const waitFor = async (predicate, label) => {
          for (let attempt = 0; attempt < 100; attempt += 1) {
            if (predicate()) return;
            await new Promise((resolve) => setTimeout(resolve, 20));
          }
          throw new Error("Timed out waiting for " + label);
        };
        await waitFor(() => !document.querySelector("#setup-view").hidden, "setup");
        // The build has to name itself without reaching anywhere for the answer.
        if (!document.querySelector("#build-identity").textContent.includes("build 39773037b5d5")) {
          throw new Error("The running build does not identify itself");
        }

        // The update check has to be something the operator SAW, not something
        // they are asked to take on trust, and nothing may download until they
        // choose to.
        const updateStrip = document.querySelector("#update-strip");
        const installUpdate = document.querySelector("#install-update");
        await waitFor(
          () => updateStrip.dataset.state === "available",
          "update check result",
        );
        if (updateStrip.hidden) {
          throw new Error("the update check ran without saying so on screen");
        }
        if (
          updateStrip.getAttribute("role") !== "status" ||
          updateStrip.getAttribute("aria-live") !== "polite" ||
          updateStrip.getAttribute("aria-atomic") !== "true"
        ) {
          throw new Error("the complete update announcement is not one atomic live region");
        }
        if (
          !updateStrip.textContent.includes("0.2.0") ||
          !updateStrip.textContent.includes("0.1.0")
        ) {
          throw new Error(
            "the update offer did not name both versions: " + updateStrip.textContent,
          );
        }
        if (!updateStrip.textContent.includes("Nothing has been downloaded")) {
          throw new Error("the update offer did not say that nothing was downloaded yet");
        }
        if (installUpdate.hidden) {
          throw new Error("an available update offered no way to consent to it");
        }
        installUpdate.focus();
        if (document.activeElement !== installUpdate) {
          throw new Error("the update consent control could not receive focus");
        }
        installUpdate.click();
        await waitFor(
          () => updateStrip.dataset.state === "installed",
          "update install result",
        );
        if (!installUpdate.hidden) {
          throw new Error("the install button survived the install it performed");
        }
        if (document.activeElement !== updateStrip) {
          throw new Error("focus was lost when the install button disappeared");
        }
        if (!updateStrip.textContent.includes("0.2.0")) {
          throw new Error("the live update result omitted the installed version");
        }
        const setupButtonBounds = document
          .querySelector("#add-locations")
          .getBoundingClientRect()
          .toJSON();
        document.querySelector("#add-locations").click();
        await waitFor(() => document.querySelectorAll("#location-list li").length === 1, "location");
        // A folder folded into one that already covers it is accounted for on
        // screen. Silence here was the bug: the owner picked those folders
        // deliberately, and saying nothing reads as the app ignoring them.
        if (!document.querySelector("#setup-status").textContent.includes("already inside a folder you approved")) {
          throw new Error("The folded location was not accounted for");
        }
        // Folding silently un-skipped a folder: the index then reads what the
        // owner deliberately excluded, and the first sign is the wait.
        if (!document.querySelector("#setup-status").textContent.includes("because you just chose")) {
          throw new Error("A skip cancelled by the owner's own choice was not announced");
        }
        if (!document.querySelector("#setup-status").textContent.includes("was folded in")) {
          throw new Error("A skipped folder was forgotten without saying so");
        }
        // Skipping folders is reachable only once a location exists, and it
        // must report both what it skipped and what it could not.
        if (document.querySelector("#skip-folders").disabled) {
          throw new Error("Skip folders stayed disabled with a location approved");
        }
        document.querySelector("#skip-folders").click();
        await waitFor(
          () => document.querySelector("#setup-status").textContent.includes("will be skipped"),
          "skipped folders",
        );
        if (!document.querySelector("#setup-status").textContent.includes("3 folders")) {
          throw new Error("The skipped count came from the interface, not the native side");
        }
        if (!document.querySelector("#setup-status").textContent.includes("not inside a folder you chose")) {
          throw new Error("A folder outside every approved location was dropped silently");
        }
        // The way back must be visible once something is skipped.
        if (document.querySelector("#clear-skipped").hidden) {
          throw new Error("Skipped folders cannot be undone from the interface");
        }
        document.querySelector("#clear-skipped").click();
        await waitFor(
          () => document.querySelector("#setup-status").textContent.includes("read whole"),
          "cleared skipped folders",
        );
        document.querySelector("#run-census").click();
        await waitFor(() => !document.querySelector("#results-view").hidden, "census result");
        if (!document.querySelector("#result-summary").textContent.includes("without opening")) {
          throw new Error("Census privacy copy is missing");
        }
        document.querySelector("#build-text-vault").click();
        await waitFor(() => !document.querySelector("#search-view").hidden, "search view");
        const query = document.querySelector("#search-query");
        query.value = "Find confidentiality provisions under three sentences covering affiliates.";
        document.querySelector("#search-form").dispatchEvent(
          new Event("submit", { bubbles: true, cancelable: true })
        );
        await waitFor(() => document.querySelectorAll(".evidence-card").length === 2, "evidence");
        const body = document.body.innerText;
        if (
          !body.includes("Synthetic Agreement") ||
          !body.includes("Checked for this search") ||
          !body.includes("when this search ran") ||
          !body.toLowerCase().includes("not exact matches") ||
          !body.includes("Closing this window forgets everything") ||
          // The footer used to claim networking was disabled outright. It is
          // not, and the corrected claim is load-bearing for the disclosure
          // Peter is given.
          !body.includes("Updates only before folders open")
        ) {
          throw new Error("Evidence provenance or session-disposal notice did not render");
        }
        const documentWarning = document.querySelector(".document-evidence-warning");
        if (
          !documentWarning ||
          !documentWarning.textContent.includes("Some matching passages are not shown") ||
          !documentWarning.textContent.includes("may be supported by a passage")
        ) {
          throw new Error("A cut document card did not disclose its missing passages");
        }
        // A transcription must render as its own thing: never inside an
        // exact-excerpt element, and always carrying its confidence.
        // Every card must offer a way back to the source.
        const revealButtons = document.querySelectorAll(".reveal-source");
        if (revealButtons.length < 2) {
          throw new Error("cards did not offer a way to show the source in Finder");
        }

        const transcriptionCards = document.querySelectorAll(".transcription-card");
        if (transcriptionCards.length !== 1) {
          throw new Error("the transcription did not render as its own card");
        }
        const transcription = transcriptionCards[0];
        if (!transcription.querySelector(".transcribed-text")) {
          throw new Error("a transcription rendered without its transcription styling");
        }
        if (transcription.classList.contains("evidence-card")) {
          throw new Error("a transcription carried the evidence-card class");
        }
        if (transcription.querySelector(".evidence-excerpt")) {
          throw new Error("a transcription rendered as an exact excerpt");
        }
        if (!transcription.textContent.includes("62%")) {
          throw new Error("a transcription rendered without its confidence");
        }
        // Asserted on the heading element, not on the page. The card's own
        // disclosure carries the same words, so a body-wide check passed even
        // with the heading replaced by "Results".
        const transcriptionHeading = document.querySelector(".transcription-heading");
        if (!transcriptionHeading || !transcriptionHeading.textContent.includes("machine's reading")) {
          throw new Error("the transcription heading did not disclose what it is");
        }

        // The wait must be stated before anyone commits to it.
        const forecast = document.querySelector("#build-forecast");
        if (!forecast || forecast.hidden) {
          throw new Error("no build-time forecast was shown for a folder of 400 scans");
        }
        if (!forecast.textContent.includes("400 scans")) {
          throw new Error("the forecast did not say what dominates: " + forecast.textContent);
        }

        const vaultSummary = document.querySelector("#vault-summary").textContent;
        // Fails if the UI again implies that any provision in a scan-bearing PDF is quotable.
        if (
          !vaultSummary.includes("aliases or shortcuts (2)") ||
          !vaultSummary.includes("have a second name elsewhere on the disk (3)") ||
          !vaultSummary.includes("9,023 items could not be read") ||
          !vaultSummary.includes("4 documents contain text read from scans") ||
          !vaultSummary.includes("an imported PDF containing any page scan is not quotable at all") ||
          !vaultSummary.includes("This index is PARTIAL") ||
          !vaultSummary.includes("5,000 documents were not read") ||
          !vaultSummary.includes("3 folders were too deep to enter") ||
          !vaultSummary.includes("blocked by macOS permissions (9,000)") ||
          !vaultSummary.includes("scans the text reader could not make out (12)") ||
          !vaultSummary.includes("changed while being read (2)") ||
          !vaultSummary.includes("7 passages were not prepared for suggestions") ||
          !vaultSummary.includes("2 folders were skipped at your request") ||
          !vaultSummary.includes("1 skipped folder was moved or replaced")
        ) {
          throw new Error("Skipped links are not disclosed: " + vaultSummary);
        }
        if (body.includes("/Users/") || body.includes("SYNTHETIC_CONTENT_CANARY")) {
          throw new Error("A path or source canary crossed the UI boundary");
        }
        return {
          locations: document.querySelectorAll("#location-list li").length,
          evidenceCards: document.querySelectorAll(".evidence-card").length,
          searchVisible: !document.querySelector("#search-view").hidden,
          setupButtonBounds,
        };
      })()
    `,
  });
  if (smoke.exceptionDetails) {
    throw new Error(smoke.exceptionDetails.exception?.description ?? "UI smoke failed");
  }
  const image = await cdp.send("Page.captureScreenshot", {
    format: "png",
    captureBeyondViewport: false,
  });
  await writeFile(screenshot, Buffer.from(image.data, "base64"));
  await cdp.send("Runtime.evaluate", {
    expression: "window.scrollTo({ top: 0, behavior: 'instant' })",
  });
  const updateImage = await cdp.send("Page.captureScreenshot", {
    format: "png",
    captureBeyondViewport: false,
  });
  await writeFile(updateScreenshot, Buffer.from(updateImage.data, "base64"));
  cdp.close();
  process.stdout.write(
    `${JSON.stringify(
      {
        ...smoke.result.value,
        ...(process.env.ARCHIVE_KEEP_UI_SMOKE
          ? { screenshot, updateScreenshot }
          : {}),
      },
      null,
      2,
    )}\n`,
  );
} finally {
  if (browser.exitCode === null) {
    const exited = new Promise((resolveExit) => browser.once("exit", resolveExit));
    browser.kill("SIGTERM");
    await Promise.race([
      exited,
      new Promise((resolveWait) => setTimeout(resolveWait, 1000)),
    ]);
  }
  if (!process.env.ARCHIVE_KEEP_UI_SMOKE) {
    for (let attempt = 0; attempt < 10; attempt += 1) {
      try {
        await rm(profile, { recursive: true, force: true });
        break;
      } catch (error) {
        if (attempt === 9) throw error;
        await new Promise((resolveWait) => setTimeout(resolveWait, 50));
      }
    }
  }
}
