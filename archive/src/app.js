const invoke = window.__TAURI__.core.invoke;

const elements = {
  setupView: document.querySelector("#setup-view"),
  scanningView: document.querySelector("#scanning-view"),
  resultsView: document.querySelector("#results-view"),
  indexingView: document.querySelector("#indexing-view"),
  searchView: document.querySelector("#search-view"),
  emptyLocations: document.querySelector("#empty-locations"),
  locationList: document.querySelector("#location-list"),
  addLocations: document.querySelector("#add-locations"),
  skipFolders: document.querySelector("#skip-folders"),
  clearSkipped: document.querySelector("#clear-skipped"),
  runCensus: document.querySelector("#run-census"),
  cancelCensus: document.querySelector("#cancel-census"),
  cancelIndexing: document.querySelector("#cancel-indexing"),
  indexingProgress: document.querySelector("#indexing-progress"),
  startOver: document.querySelector("#start-over"),
  exportReport: document.querySelector("#export-report"),
  buildTextVault: document.querySelector("#build-text-vault"),
  backToCensus: document.querySelector("#back-to-census"),
  setupStatus: document.querySelector("#setup-status"),
  exportStatus: document.querySelector("#export-status"),
  errorBanner: document.querySelector("#error-banner"),
  errorMessage: document.querySelector("#error-message"),
  dismissError: document.querySelector("#dismiss-error"),
  resultStatus: document.querySelector("#result-status"),
  resultSummary: document.querySelector("#result-summary"),
  metricArtifacts: document.querySelector("#metric-artifacts"),
  metricLocations: document.querySelector("#metric-locations"),
  metricBytes: document.querySelector("#metric-bytes"),
  metricCloud: document.querySelector("#metric-cloud"),
  categoryList: document.querySelector("#category-list"),
  signalsList: document.querySelector("#signals-list"),
  vaultSummary: document.querySelector("#vault-summary"),
  buildForecast: document.querySelector("#build-forecast"),
  searchForm: document.querySelector("#search-form"),
  searchQuery: document.querySelector("#search-query"),
  searchSubmit: document.querySelector("#search-submit"),
  queryInterpretation: document.querySelector("#query-interpretation"),
  queryChips: document.querySelector("#query-chips"),
  candidateCount: document.querySelector("#candidate-count"),
  searchStatus: document.querySelector("#search-status"),
  searchResults: document.querySelector("#search-results"),
  buildIdentity: document.querySelector("#build-identity"),
};

let locations = [];
let lastReport = null;
let vaultReport = null;

const categoryLabels = {
  pdf: "PDF",
  word_processing: "Word processing",
  email: "Email containers",
  plain_text: "Plain text & Markdown",
  markup: "HTML & XML (not searchable)",
  image_or_scan: "Images & scans",
  spreadsheet: "Spreadsheets",
  presentation: "Presentations",
  archive: "Compressed archives",
  database: "Databases",
  apple_document: "Apple documents",
  icloud_placeholder: "Still in iCloud",
  other: "Other formats",
};

function showError(error) {
  elements.errorMessage.textContent =
    typeof error === "string" ? error : "The operation could not be completed safely.";
  elements.errorBanner.hidden = false;
}

function hideError() {
  elements.errorBanner.hidden = true;
  elements.errorMessage.textContent = "";
}

function showView(name) {
  elements.setupView.hidden = name !== "setup";
  elements.scanningView.hidden = name !== "scanning";
  elements.resultsView.hidden = name !== "results";
  elements.indexingView.hidden = name !== "indexing";
  elements.searchView.hidden = name !== "search";
  const activeStep =
    name === "setup" ? "1" : name === "scanning" ? "2" : name === "results" ? "3" : "4";
  for (const step of document.querySelectorAll("[data-step]")) {
    step.classList.toggle("is-active", step.dataset.step === activeStep);
  }
  // Past step 1 the operator is working, not being introduced.
  document.body.classList.toggle("is-working", activeStep !== "1");
}

function setLocationControlsDisabled(disabled) {
  elements.addLocations.disabled = disabled;
  // Nothing to skip until there is a location to skip inside of.
  elements.skipFolders.disabled = disabled || locations.length === 0;
  elements.clearSkipped.disabled = disabled;
  elements.runCensus.disabled = disabled || locations.length === 0;
  for (const button of elements.locationList.querySelectorAll("button")) {
    button.disabled = disabled;
  }
}

function renderLocations(nextLocations) {
  locations = nextLocations;
  elements.locationList.replaceChildren();
  elements.emptyLocations.hidden = locations.length > 0;

  for (const location of locations) {
    const item = document.createElement("li");
    item.className = "location-item";

    const copy = document.createElement("div");
    copy.className = "location-copy";
    const icon = document.createElement("span");
    icon.className = "location-icon";
    icon.setAttribute("aria-hidden", "true");
    // An inline folder glyph rather than a text character: the shipped fonts
    // carry no U+2302 HOUSE, so the webview substituted a fallback font and
    // drew a misaligned sliver in the icon box.
    icon.innerHTML =
      '<svg width="16" height="16" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">' +
      '<path d="M1.75 4a1 1 0 0 1 1-1h3.3l1.5 1.8h5.7a1 1 0 0 1 1 1V12a1 1 0 0 1-1 1h-10.5a1 1 0 0 1-1-1z" ' +
      'stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"/></svg>';
    const text = document.createElement("div");
    const title = document.createElement("strong");
    title.textContent = location.label;
    const detail = document.createElement("span");
    // Counts, when a census has produced them, are what tells one opaque row
    // from another. They name nothing: a folder of exhibits and a matter
    // archive differ by two orders of magnitude and by nothing identifying.
    detail.textContent =
      typeof location.artifacts === "number"
        ? `${location.artifacts.toLocaleString()} item${
            location.artifacts === 1 ? "" : "s"
          } · ${formatBytes(location.regularFileBytes ?? 0)} · read-only`
        : "Read-only";
    text.append(title, detail);
    copy.append(icon, text);

    // Finder answers "which folder is this?" better than any label the
    // interface could carry, and without a path crossing into it.
    const reveal = document.createElement("button");
    reveal.className = "button button-quiet reveal-location";
    reveal.type = "button";
    reveal.textContent = "Show in Finder";
    reveal.setAttribute("aria-label", `Show ${location.label} in Finder`);
    reveal.addEventListener("click", async () => {
      reveal.disabled = true;
      try {
        await invoke("reveal_archive_location", { locationId: location.id });
      } catch (error) {
        showError(String(error));
      } finally {
        reveal.disabled = false;
      }
    });

    const remove = document.createElement("button");
    remove.className = "remove-location";
    remove.type = "button";
    remove.setAttribute("aria-label", `Remove ${location.label}`);
    remove.textContent = "×";
    remove.addEventListener("click", () => removeLocation(location.id));

    const actions = document.createElement("div");
    actions.className = "location-actions";
    actions.append(reveal, remove);
    item.append(copy, actions);
    elements.locationList.append(item);
  }

  elements.runCensus.disabled = locations.length === 0;
  elements.skipFolders.disabled = locations.length === 0;
  elements.setupStatus.textContent =
    locations.length === 0
      ? "Choose at least one folder to continue."
      : `${locations.length.toLocaleString()} folder${
          locations.length === 1 ? "" : "s"
        } chosen. Nothing has been looked at yet.`;
}

async function chooseLocations() {
  hideError();
  setLocationControlsDisabled(true);
  try {
    const choice = await invoke("choose_archive_locations");
    renderLocations(choice.locations);
    // Folders already covered by an approved location are folded in rather
    // than refused. Say so plainly: they were chosen on purpose, and silence
    // here reads as the app having ignored them.
    if (choice.folded > 0) {
      elements.setupStatus.textContent = `${elements.setupStatus.textContent} ${
        choice.folded === 1 ? "One folder was" : `${choice.folded} folders were`
      } already inside a folder you approved, so ${
        choice.folded === 1 ? "it is" : "they are"
      } covered by that one.`;
    }
    // A chosen folder beats an older skip, and the cancelled skip is said out
    // loud: the skip was deliberate once too, and its silent disappearance
    // would be the same lie in the other direction.
    if (choice.unskipped > 0) {
      elements.setupStatus.textContent = `${elements.setupStatus.textContent} ${
        choice.unskipped === 1
          ? "One folder you had skipped is"
          : `${choice.unskipped} folders you had skipped are`
      } no longer skipped, because you just chose ${
        choice.unskipped === 1 ? "it" : "them"
      }.`;
    }
    // Folding a location forgets the folders skipped inside it. That reads
    // more than was asked for, never less, so nothing goes missing from the
    // index -- but those folders were excluded deliberately, and finding out
    // by watching an extra seventeen minutes of text recognition go by is not
    // finding out.
    if (choice.forgottenSkips > 0) {
      elements.setupStatus.textContent = `${elements.setupStatus.textContent} ${
        choice.forgottenSkips === 1
          ? "One folder you had skipped is"
          : `${choice.forgottenSkips} folders you had skipped are`
      } no longer skipped, because the location holding ${
        choice.forgottenSkips === 1 ? "it" : "them"
      } was folded in. Choose them again under "Skip folders" if you still want ${
        choice.forgottenSkips === 1 ? "it" : "them"
      } left out.`;
    }
    lastReport = null;
    vaultReport = null;
  } catch (error) {
    showError(error);
  } finally {
    setLocationControlsDisabled(false);
  }
}

/// Folders chosen here are never read during a build.
///
/// The panel is the same native one used for locations, so the interface still
/// receives no path -- only counts. Every outcome is reported, including the
/// ones that did nothing: an operator who believes a folder is being skipped
/// and is wrong ends up with an index they cannot account for.
async function chooseSkippedFolders() {
  hideError();
  setLocationControlsDisabled(true);
  try {
    const choice = await invoke("choose_archive_exclusions");
    // Skipping is reversible, and the way back has to be visible: a folder
    // skipped by mistake is a folder silently missing from every search.
    elements.clearSkipped.hidden = choice.total === 0;
    const notes = [];
    if (choice.skipped > 0) {
      notes.push(
        `${choice.total.toLocaleString()} folder${
          choice.total === 1 ? "" : "s"
        } will be skipped when the index is built.`,
      );
    }
    if (choice.outside > 0) {
      notes.push(
        `${choice.outside.toLocaleString()} ${
          choice.outside === 1 ? "folder is" : "folders are"
        } not inside a folder you chose, so ${
          choice.outside === 1 ? "it was" : "they were"
        } not added.`,
      );
    }
    if (choice.refusedWholeLocation > 0) {
      notes.push(
        "You cannot skip a whole folder you chose — remove it instead.",
      );
    }
    if (notes.length > 0) {
      elements.setupStatus.textContent = notes.join(" ");
    }
    lastReport = null;
    vaultReport = null;
  } catch (error) {
    showError(error);
  } finally {
    setLocationControlsDisabled(false);
  }
}

async function clearSkippedFolders() {
  hideError();
  setLocationControlsDisabled(true);
  try {
    const cleared = await invoke("clear_archive_exclusions");
    elements.clearSkipped.hidden = true;
    elements.setupStatus.textContent = `${cleared.toLocaleString()} skipped folder${
      cleared === 1 ? "" : "s"
    } restored. Every folder you chose will be read whole.`;
    lastReport = null;
    vaultReport = null;
  } catch (error) {
    showError(error);
  } finally {
    setLocationControlsDisabled(false);
  }
}

async function removeLocation(locationId) {
  hideError();
  setLocationControlsDisabled(true);
  try {
    renderLocations(await invoke("remove_archive_location", { locationId }));
    lastReport = null;
    vaultReport = null;
  } catch (error) {
    showError(error);
  } finally {
    setLocationControlsDisabled(false);
  }
}

function formatBytes(bytes) {
  if (bytes === 0) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** index;
  return `${value >= 10 || index === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[index]}`;
}

function createSignal(label, value) {
  const row = document.createElement("div");
  const term = document.createElement("dt");
  term.textContent = label;
  const description = document.createElement("dd");
  description.textContent = Number(value).toLocaleString();
  row.append(term, description);
  return row;
}

// Say how long this will take BEFORE anyone commits to waiting.
//
// The census already knows the counts, so there is no reason to discover the
// cost an hour in. The estimate is deliberately coarse and labelled as one:
// scans dominate, each needing its own recognizer process, and a wrong precise
// number is worse than an honest rough one.
function renderBuildForecast(report) {
  const forecast = elements.buildForecast;
  if (!forecast) return;
  const categories = report.categories ?? [];
  const countFor = (name) =>
    categories.find((category) => category.category === name)?.artifacts ?? 0;
  const scans = countFor("image_or_scan");
  const pdfs = countFor("pdf");
  const wordProcessing = countFor("word_processing");
  // Calibrated against a full-scale run, not a per-page guess: 16,621
  // documents in that archive's own format mix -- 2,774 scans, 447 PDFs, 349
  // Word -- took 1,028 seconds end to end. Extraction runs in parallel, which
  // the old per-page arithmetic ignored, so it predicted 25 minutes for a
  // build that takes 17. These rates keep a little headroom over the measured
  // figure; a build that finishes early is a better surprise than one that
  // runs past its own estimate.
  const seconds = scans * 0.3 + (pdfs + wordProcessing) * 0.4;
  if (seconds < 60) {
    forecast.hidden = true;
    return;
  }
  const minutes = Math.max(1, Math.round(seconds / 60));
  const parts = [];
  if (scans > 0) parts.push(`${scans.toLocaleString()} scans`);
  if (pdfs > 0) parts.push(`${pdfs.toLocaleString()} PDFs`);
  if (wordProcessing > 0) parts.push(`${wordProcessing.toLocaleString()} Word documents`);
  forecast.textContent =
    `Roughly ${minutes} minute${minutes === 1 ? "" : "s"} to build, mostly ` +
    `${parts.join(", ")}. Each is opened in its own sandboxed process. ` +
    `The index is held in memory and rebuilt each session.`;
  forecast.hidden = false;
}

function renderReport(report) {
  lastReport = report;
  const status = report.status.replaceAll("_", " ");
  elements.resultStatus.textContent = status;
  elements.resultStatus.dataset.status = report.status;
  elements.resultSummary.textContent =
    report.status === "complete"
      ? "Your folders were counted without opening a single document."
      : report.status === "cancelled"
        ? "Cancelled. The partial counts were thrown away."
        : "Counting stopped at a safety limit, so these totals are incomplete. See what needs attention.";

  renderBuildForecast(report);
  elements.metricArtifacts.textContent = report.summary.artifacts.toLocaleString();
  elements.metricLocations.textContent = report.summary.approved_locations.toLocaleString();
  elements.metricBytes.textContent = formatBytes(report.summary.regular_file_bytes);
  elements.metricCloud.textContent = report.signals.icloud_placeholders.toLocaleString();

  elements.categoryList.replaceChildren();
  const categories = [...report.categories].sort((left, right) => right.artifacts - left.artifacts);
  const largest = Math.max(1, ...categories.map((category) => category.artifacts));
  for (const category of categories.slice(0, 10)) {
    const row = document.createElement("div");
    row.className = "category-row";
    const label = document.createElement("span");
    label.className = "category-label";
    label.textContent = categoryLabels[category.category] ?? category.category;
    const track = document.createElement("span");
    track.className = "category-track";
    const fill = document.createElement("span");
    fill.style.width = `${Math.max(2, (category.artifacts / largest) * 100)}%`;
    track.append(fill);
    const count = document.createElement("span");
    count.className = "category-count";
    count.textContent = category.artifacts.toLocaleString();
    row.append(label, track, count);
    elements.categoryList.append(row);
  }
  if (categories.length === 0) {
    const empty = document.createElement("p");
    empty.className = "status";
    empty.textContent = "No files were found.";
    elements.categoryList.append(empty);
  }

  elements.signalsList.replaceChildren(
    createSignal(
      "Scans that need reading",
      report.categories.find((item) => item.category === "image_or_scan")?.artifacts ?? 0,
    ),
    createSignal("Not downloaded from iCloud", report.signals.icloud_placeholders),
    createSignal("Pages, Numbers, Keynote", report.summary.packages),
    createSignal("Blocked by permissions", report.signals.permission_mode_unreadable),
    createSignal("Shortcuts skipped", report.signals.symlinks_skipped),
    createSignal(
      "Could not be examined",
      report.signals.metadata_errors + report.signals.directory_errors,
    ),
  );

  const usableReport = report.status === "complete" || report.status === "partial";
  elements.exportReport.disabled = !usableReport;
  elements.buildTextVault.disabled = !usableReport;
  elements.exportStatus.textContent = "";
  showView("results");
}

async function runCensus() {
  hideError();
  vaultReport = null;
  showView("scanning");
  try {
    const report = await invoke("run_archive_census");
    if (report.status === "cancelled") {
      lastReport = null;
      showView("setup");
      elements.setupStatus.textContent = "Cancelled. Nothing was kept.";
      return;
    }
    renderReport(report);
  } catch (error) {
    showView("setup");
    showError(error);
  }
}

async function cancelOperation(button) {
  button.disabled = true;
  const original = button.textContent;
  button.textContent = "Cancelling…";
  try {
    await invoke("cancel_archive_census");
  } catch (error) {
    showError(error);
  } finally {
    button.textContent = original;
    button.disabled = false;
  }
}

async function exportReport() {
  hideError();
  elements.exportReport.disabled = true;
  elements.exportStatus.textContent = "Preparing report…";
  try {
    const saved = await invoke("export_archive_census");
    elements.exportStatus.textContent = saved ? "Report saved." : "";
  } catch (error) {
    elements.exportStatus.textContent = "";
    showError(error);
  } finally {
    elements.exportReport.disabled = false;
  }
}

function renderVaultSummary(report) {
  vaultReport = report;
  // Every fact below was already disclosed; it was disclosed as one fifteen-line
  // paragraph, which is the same as not disclosing it. An attorney opening this
  // screen needs to know in one glance what is searchable, and then be able to
  // find the caveat that applies to them. Nothing is dropped, and the warnings
  // that change whether a negative result can be trusted are marked as such.
  const lead = `${report.indexed_documents.toLocaleString()} document${
    report.indexed_documents === 1 ? "" : "s"
  } ready to search (${formatBytes(report.indexed_bytes)}). Held in memory only — nothing was saved to disk.`;

  const notes = [];
  const formatBreakdown = [
    `${report.searchable_pdf_documents.toLocaleString()} PDF`,
    // One number for every word-processor format. The distinction that matters
    // to counsel is how much became searchable, not which container it arrived in.
    `${report.docx_documents.toLocaleString()} Word or OpenDocument`,
  ].join(", ");
  notes.push({ text: `You can search: ${formatBreakdown}.` });

  // Its own line. A document with machine-read text is searchable, but only as
  // a transcription, and folding it into the indexed total would say the
  // archive can quote more of itself than it can.
  if ((report.transcribed_documents ?? 0) > 0) {
    notes.push({
      text: `${report.transcribed_documents.toLocaleString()} document${
        report.transcribed_documents === 1 ? "" : "s"
      } contain text read from scans — you can search the machine reading, but none of those documents is quotable as the document\u0027s own words.`,
    });
  }
  const mixedProvenanceDocuments = report.mixed_provenance_documents ?? 0;
  if (mixedProvenanceDocuments > 0) {
    notes.push({
      text: `${mixedProvenanceDocuments.toLocaleString()} document${
        mixedProvenanceDocuments === 1 ? " has" : "s have"
      } both extracted and transcribed provisions. Only separately extracted provisions can be quoted; an imported PDF containing any page scan is not quotable at all.`,
    });
  }
  if (report.unsupported_files_skipped > 0 || report.ocr_required_files > 0) {
    notes.push({
      text: `${report.unsupported_files_skipped.toLocaleString()} file${
        report.unsupported_files_skipped === 1 ? "" : "s"
      } this app cannot open ${
        report.unsupported_files_skipped === 1 ? "was" : "were"
      } skipped; ${report.ocr_required_files.toLocaleString()} PDF${
        report.ocr_required_files === 1 ? "" : "s"
      } are pictures of pages with no text to read.`,
    });
  }
  // Folders the owner told the build not to enter. Deliberate, so not a
  // warning -- but it is a coverage gap, and the reader weeks later (or a
  // different reader entirely) was not in the room when the skip was chosen.
  if ((report.excluded_directories ?? 0) > 0) {
    notes.push({
      text: `${report.excluded_directories.toLocaleString()} folder${
        report.excluded_directories === 1 ? " was" : "s were"
      } skipped at your request; nothing inside ${
        report.excluded_directories === 1 ? "it" : "them"
      } is in this index.`,
    });
  }
  if ((report.excluded_folder_changes ?? 0) > 0) {
    notes.push({
      warning: true,
      text: `${report.excluded_folder_changes.toLocaleString()} skipped folder${
        report.excluded_folder_changes === 1 ? " was" : "s were"
      } moved or replaced. Minutes did not transfer your choice to a different folder with the old name, and kept the original folder out when it could identify it. Review your folders before relying on what search does not find.`,
    });
  }
  // Defaulted, not asserted: a missing count must never blank the search view.
  const inferredBoundaryDocuments = report.inferred_boundary_documents ?? 0;
  if (inferredBoundaryDocuments > 0) {
    notes.push({
      text: `${inferredBoundaryDocuments.toLocaleString()} document${
        inferredBoundaryDocuments === 1 ? "" : "s"
      } cannot answer "in the same clause" questions, because the file does not record where one clause ends and the next begins.`,
    });
  }
  notes.push({
    text: report.semantic_retrieval_enabled
      ? `Suggestions are on: ${report.semantic_provisions_indexed.toLocaleString()} passage${
          report.semantic_provisions_indexed === 1 ? "" : "s"
        } were prepared using the language model built into macOS, on this Mac.`
      : "Suggestions by meaning are not available on this Mac. Exact search still works.",
  });
  // Partial suggestion coverage is not the same as none. Exact search is
  // complete either way; a reader who assumes the suggestions swept the whole
  // archive would be wrong.
  if (report.semantic_coverage_partial) {
    const skippedSuggestions = report.semantic_provisions_skipped ?? 0;
    notes.push({
      warning: true,
      text: `Suggestions cover only part of your documents. ${skippedSuggestions.toLocaleString()} passage${
        skippedSuggestions === 1 ? " was" : "s were"
      } not prepared for suggestions. Exact search covers all of your documents.`,
    });
  }
  const dropped = describeDroppedSources(report).trim();
  if (dropped) {
    notes.push({ text: dropped });
  }
  // A partial index must say so on its face. It is a real index and worth
  // having, but a reader who thinks it covers the whole folder will draw a
  // false conclusion from a negative result.
  if (report.budget_reached) {
    notes.push({
      warning: true,
      text:
        `This index is PARTIAL: a limit was reached. ${(
          report.documents_left_unread ?? 0
        ).toLocaleString()} document${
          report.documents_left_unread === 1 ? " was" : "s were"
        } not read` +
        ((report.directories_left_unread ?? 0) > 0
          ? `, and ${report.directories_left_unread.toLocaleString()} folder${
              report.directories_left_unread === 1 ? " was" : "s were"
            } too deep to enter`
          : "") +
        ". Narrow the approved folders and rebuild before relying on a negative result.",
    });
  }

  elements.vaultSummary.replaceChildren();
  const leadLine = document.createElement("p");
  leadLine.className = "vault-lead";
  leadLine.textContent = lead;
  const list = document.createElement("ul");
  list.className = "vault-notes";
  // Warnings first: they decide whether a negative result means anything.
  for (const note of [...notes].sort((left, right) => (right.warning ? 1 : 0) - (left.warning ? 1 : 0))) {
    const item = document.createElement("li");
    if (note.warning) item.className = "is-warning";
    item.textContent = note.text;
    list.append(item);
  }
  elements.vaultSummary.append(leadLine, list);
}

// Documents that could not be indexed were counted and never shown. Counsel
// saw "N supported documents indexed" and had no way to learn that a
// privileged PDF had been refused by the converter, was malformed, was over
// budget, or sat behind a permission error -- a search would simply return
// nothing for a clause that is plainly in the file. Coverage gaps have to be
// visible to be acted on.
function describeDroppedSources(report) {
  const dropped = [
    [report.conversion_failures, "could not be opened by the converter"],
    [report.malformed_text_files_skipped, "damaged"],
    [report.oversized_files_skipped, "too large"],
    [report.duplicate_files_skipped, "duplicates"],
    // Both link counters were tallied and never shown. On de-duplicated or
    // linked storage every file can carry a second name, so the whole archive
    // is refused and counsel is told only that fewer documents were indexed
    // than the folder holds -- a silent, total coverage loss.
    [report.symlinks_skipped, "aliases or shortcuts"],
    [report.hard_links_skipped, "have a second name elsewhere on the disk"],
    // Split five ways after a real archive reported 10,429 of ~16,600
    // documents "could not be read" and the number could not be acted on.
    // Permission denied is a thing a reader can fix; a file changing mid-read
    // is not the same problem at all.
    [report.permission_denied, "blocked by macOS permissions"],
    [report.unopenable, "could not be opened for some other reason"],
    [
      report.open_file_limit_reached,
      "not reached because the app ran out of open files (restart it and try a smaller folder)",
    ],
    [report.scans_unreadable, "scans the text reader could not make out"],
    [report.entries_unstattable, "could not be examined at all"],
    [report.identity_unavailable, "could not be tracked reliably"],
    [report.changed_while_reading, "changed while being read"],
    [report.directory_errors, "inside folders that could not be read"],
  ].filter(([count]) => count > 0);
  if (dropped.length === 0) {
    return "Every supported document was read. ";
  }
  const total = dropped.reduce((sum, [count]) => sum + count, 0);
  // "reason (count)", not "count reason": the reasons are phrases that cannot
  // agree with every number, and "1 were aliases or shortcuts" is the kind of
  // sentence that makes a reader distrust the number beside it.
  const detail = dropped
    .map(([count, reason]) => `${reason} (${count.toLocaleString()})`)
    .join(", ");
  // "items", not "documents": `directory_errors` counts folders, and an alias
  // can point at one, so the aggregate spans more than files. Calling it a
  // document count would overstate how many documents are missing.
  return (
    `${total.toLocaleString()} item${total === 1 ? "" : "s"} ` +
    `could not be read, so ${total === 1 ? "it is" : "they are"} not searchable: ` +
    `${detail}. Check these before you trust a "nothing found" result. `
  );
}

let indexingPoll = null;

// Polled, not pushed. Two integers on a timer need no event channel, and a
// channel opened for progress is a channel that could later carry something
// else.
function startIndexingProgress() {
  const started = Date.now();
  stopIndexingProgress();
  const tick = async () => {
    try {
      const progress = await invoke("archive_index_progress");
      const seconds = Math.max(1, Math.round((Date.now() - started) / 1000));
      const rate = progress.examined / seconds;
      // No percentage. The total is not known until the walk finishes, and a
      // made-up denominator would be a number the reader could act on and
      // should not.
      const parts = [
        `${progress.indexed.toLocaleString()} indexed`,
        `${progress.examined.toLocaleString()} examined`,
        `${formatElapsed(Date.now() - started)} elapsed`,
      ];
      if (rate >= 0.2) {
        parts.push(`~${Math.round(rate)}/s`);
      }
      elements.indexingProgress.textContent = parts.join(" · ");
    } catch {
      // A failed poll is not worth surfacing; the build reports its own errors.
    }
  };
  tick();
  indexingPoll = setInterval(tick, 700);
}

function stopIndexingProgress() {
  if (indexingPoll !== null) {
    clearInterval(indexingPoll);
    indexingPoll = null;
  }
}

function formatElapsed(milliseconds) {
  const total = Math.round(milliseconds / 1000);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return minutes > 0 ? `${minutes}m ${seconds}s` : `${seconds}s`;
}

async function buildTextVault() {
  hideError();
  showView("indexing");
  startIndexingProgress();
  try {
    const report = await invoke("build_archive_text_vault");
    renderVaultSummary(report);
    elements.searchResults.replaceChildren();
    elements.queryInterpretation.hidden = true;
    elements.searchStatus.textContent =
      report.indexed_documents === 0
        ? "No searchable PDF, Word, OpenDocument, RTF, TXT, or Markdown documents were found in the approved locations."
        : "Type what you are looking for. You get exact quotes from your own documents, not an answer written by an AI.";
    showView("search");
    if (report.indexed_documents > 0) {
      elements.searchQuery.focus();
    }
  } catch (error) {
    if (lastReport) renderReport(lastReport);
    else showView("setup");
    showError(error);
  } finally {
    // `finally`, so a build that errors does not leave a timer polling a
    // command whose state has moved on.
    stopIndexingProgress();
  }
}

function humanize(value) {
  return value.replaceAll("_", " ");
}

function addQueryChip(label) {
  const chip = document.createElement("span");
  chip.className = "query-chip";
  chip.textContent = label;
  elements.queryChips.append(chip);
}

function renderQueryInterpretation(response) {
  const query = response.query;
  elements.queryChips.replaceChildren();
  addQueryChip(
    query.scope === "same_provision" ? "All in one clause" : "Anywhere in one document",
  );
  for (const concept of query.required_concepts) {
    addQueryChip(`Must mention: ${humanize(concept)}`);
  }
  for (const concept of query.excluded_concepts) {
    addQueryChip(`Exclude: ${humanize(concept)}`);
  }
  if (query.exact_phrase) addQueryChip(`Exact: “${query.exact_phrase}”`);
  if (query.max_sentences) addQueryChip(`At most ${query.max_sentences} sentences`);
  if (response.semantic_query_applied) addQueryChip("Close matches too");
  elements.candidateCount.textContent =
    `${response.lexical_candidates_considered.toLocaleString()} passage${
      response.lexical_candidates_considered === 1 ? "" : "s"
    } checked word by word` +
    (response.semantic_query_applied
      ? ` · ${response.semantic_candidates_considered.toLocaleString()} compared by meaning`
      : "");
  elements.queryInterpretation.hidden = false;
}

function evidenceCard(card, compact = false, semantic = false) {
  if (compact) {
    const item = document.createElement("div");
    item.className = "criterion-evidence";
    const heading = document.createElement("strong");
    heading.textContent = card.provision_heading ?? card.source_anchor;
    const excerpt = document.createElement("p");
    excerpt.textContent = card.exact_excerpt;
    item.append(heading, excerpt);
    // The compact card dropped why_matched entirely, so in document-scope
    // results the disclosure that a concept matched in the heading rather
    // than in the quoted text never reached the reader. The anchor went too,
    // leaving nothing to verify the excerpt against in the source.
    if (card.why_matched) {
      const why = document.createElement("p");
      why.className = "criterion-why";
      why.textContent = `${card.source_anchor} — ${card.why_matched}`;
      item.append(why);
    }
    return item;
  }

  const article = document.createElement("article");
  article.className = "evidence-card";
  const header = document.createElement("div");
  header.className = "evidence-card-header";
  const titleBlock = document.createElement("div");
  const kicker = document.createElement("span");
  kicker.className = "evidence-kicker";
  kicker.textContent = semantic
    ? "Meaning-similar suggestion"
    : (card.provision_heading ?? "Provision");
  const title = document.createElement("strong");
  title.className = "evidence-title";
  title.textContent = card.document_title;
  titleBlock.append(kicker, title);
  const fresh = document.createElement("span");
  fresh.className = "metadata-pill";
  fresh.textContent = card.index_fresh ? "Checked for this search" : "Unavailable";
  header.append(titleBlock, fresh);

  const excerpt = document.createElement("blockquote");
  excerpt.className = "evidence-excerpt";
  excerpt.textContent = card.exact_excerpt;
  const meta = document.createElement("div");
  meta.className = "evidence-meta";
  const anchor = document.createElement("span");
  anchor.textContent = card.source_anchor;
  const sentences = document.createElement("span");
  sentences.textContent = `${card.sentence_count} sentence${
    card.sentence_count === 1 ? "" : "s"
  }`;
  const converter = document.createElement("span");
  converter.textContent = humanize(card.source_converter);
  meta.append(anchor, sentences, converter);
  const why = document.createElement("p");
  why.className = "evidence-why";
  why.textContent = semantic ? card.why_suggested : card.why_matched;
  article.append(header, excerpt, meta, why, revealButton(card));
  if (semantic) article.classList.add("semantic-card");
  return article;
}

// Ask for the document by id and let the Rust side find it.
//
// The interface has no path to hand over and never gains one: it sends back
// the opaque id it was given, and the resolution, the identity check and
// Finder all happen behind the boundary. If the file has moved or changed
// since it was indexed the request is refused rather than pointing at
// whatever is there now.
function revealButton(card) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "reveal-source";
  button.textContent = "Show in Finder";
  button.addEventListener("click", async () => {
    button.disabled = true;
    try {
      await invoke("reveal_archive_document", { documentId: card.document_id });
    } catch (error) {
      showError(error);
    } finally {
      button.disabled = false;
    }
  });
  return button;
}

function documentCard(card) {
  const article = document.createElement("article");
  article.className = "document-card";
  const header = document.createElement("div");
  header.className = "document-card-header";
  const titleBlock = document.createElement("div");
  const kicker = document.createElement("span");
  kicker.className = "evidence-kicker";
  kicker.textContent = "Found across the whole document";
  const title = document.createElement("strong");
  title.className = "evidence-title";
  title.textContent = card.document_title;
  titleBlock.append(kicker, title);
  const count = document.createElement("span");
  count.className = "metadata-pill";
  count.textContent = `${card.criterion_evidence.length} proof provision${
    card.criterion_evidence.length === 1 ? "" : "s"
  }`;
  header.append(titleBlock, count);
  const why = document.createElement("p");
  why.className = "evidence-why";
  why.textContent = card.why_matched;
  const criteria = document.createElement("div");
  criteria.className = "criterion-list";
  for (const evidence of card.criterion_evidence) {
    criteria.append(evidenceCard(evidence, true));
  }
  article.append(header, why);
  if (card.criterion_evidence_truncated) {
    const warning = document.createElement("p");
    warning.className = "document-evidence-warning";
    warning.textContent =
      "Some matching passages are not shown. Something you asked for may be supported by a passage that was left off this card.";
    article.append(warning);
  }
  article.append(criteria);
  return article;
}

// Deliberately not `evidenceCard` with a flag. A transcription has no exact
// excerpt and no section anchor, and giving it the same builder is how it would
// eventually acquire the same styling and be read as a quotation.
function transcribedCard(card) {
  const article = document.createElement("article");
  // Its own class, not `evidence-card`. Sharing the base class is how a
  // transcription would inherit quotation styling by default the next time
  // someone edits it.
  article.className = "transcription-card";

  const title = document.createElement("h3");
  title.textContent = card.document_title;
  article.append(title);

  const anchor = document.createElement("p");
  anchor.className = "transcription-anchor";
  anchor.textContent = `${card.page_anchor} · read from a scan`;
  article.append(anchor);

  const text = document.createElement("blockquote");
  text.className = "transcribed-text";
  text.textContent = card.transcribed_text;
  article.append(text);

  const confidence = document.createElement("p");
  confidence.className = "transcription-confidence";
  const percent = Math.round((card.lowest_line_confidence ?? 0) * 100);
  confidence.textContent = `The text reader was least sure here: ${percent}%. Check this against the page before relying on it.`;
  article.append(confidence);

  const why = document.createElement("p");
  why.className = "transcription-why";
  why.textContent = card.why_transcribed;
  article.append(why);
  article.append(revealButton(card));

  return article;
}

function renderSearchResponse(response) {
  renderQueryInterpretation(response);
  elements.searchResults.replaceChildren();
  for (const card of response.evidence) {
    elements.searchResults.append(evidenceCard(card));
  }
  for (const card of response.documents) {
    elements.searchResults.append(documentCard(card));
  }
  const verifiedCount = response.evidence.length + response.documents.length;
  if (response.semantic_suggestions.length > 0) {
    const heading = document.createElement("p");
    heading.className = "semantic-heading";
    heading.textContent = "Similar wording — read these yourself; they are not exact matches";
    elements.searchResults.append(heading);
    for (const card of response.semantic_suggestions) {
      elements.searchResults.append(evidenceCard(card, false, true));
    }
  }
  // Transcriptions render through their own builder, never `evidenceCard`.
  // A reading of a scan is not a quotation and must not look like one: no
  // exact-excerpt styling, its own heading, and the confidence on every card.
  const transcriptions = response.transcriptions ?? [];
  if (transcriptions.length > 0) {
    const heading = document.createElement("p");
    heading.className = "transcription-heading";
    heading.textContent =
      "Read from scanned images · a machine's reading, not the document's own text";
    elements.searchResults.append(heading);
    for (const card of transcriptions) {
      elements.searchResults.append(transcribedCard(card));
    }
  }
  const suggestionCount = response.semantic_suggestions.length + transcriptions.length;
  if (verifiedCount === 0 && suggestionCount === 0) {
    const empty = document.createElement("div");
    empty.className = "empty-results";
    empty.textContent =
      response.stale_evidence_withdrawn > 0
        ? "This match is out of date, so it was withheld. Open your documents again before relying on it."
        : "Nothing in your documents matched all of that. Try asking for less at once, or use different wording.";
    elements.searchResults.append(empty);
  }
  const resultKind = response.query.scope === "same_provision" ? "clause" : "document";
  const staleNote =
    response.stale_evidence_withdrawn > 0
      ? ` ${response.stale_evidence_withdrawn.toLocaleString()} stale source${
          response.stale_evidence_withdrawn === 1 ? " was" : "s were"
        } withheld because the file changed.`
      : "";
  const boundaryNote =
    (response.inferred_boundary_evidence_withdrawn ?? 0) > 0
      ? ` ${(response.inferred_boundary_evidence_withdrawn ?? 0).toLocaleString()} document${
          response.inferred_boundary_evidence_withdrawn === 1 ? " does" : "s do"
        } not record where a clause ends, so ${
          response.inferred_boundary_evidence_withdrawn === 1 ? "it was" : "they were"
        } excluded from same-clause questions.`
      : "";
  elements.searchStatus.textContent =
    `${verifiedCount.toLocaleString()} ${resultKind}${
      verifiedCount === 1 ? "" : "s"
    } matched everything you asked for. ${suggestionCount.toLocaleString()} more ${
      suggestionCount === 1 ? "is" : "are"
    } close in meaning, listed separately below.${staleNote}${boundaryNote}`;
}

async function searchVault(event) {
  event.preventDefault();
  hideError();
  const query = elements.searchQuery.value.trim();
  if (!query) {
    elements.searchStatus.textContent = "Type what you are looking for first.";
    elements.searchQuery.focus();
    return;
  }
  elements.searchSubmit.disabled = true;
  elements.searchStatus.textContent =
    "Checking exact constraints, on-device meaning candidates, and current source revisions…";
  try {
    const response = await invoke("search_archive_text_vault", { query });
    renderSearchResponse(response);
  } catch (error) {
    elements.searchStatus.textContent = "Nothing matched.";
    showError(error);
  } finally {
    elements.searchSubmit.disabled = false;
  }
}

async function startOver() {
  hideError();
  showView("setup");
  // Re-read the rows rather than reusing the ones rendered before the census.
  // The per-location counts only exist once a census has run, so the state
  // this view was left in is exactly the state that lacks them, and the rows
  // would stay unlabelled for the whole session.
  try {
    const state = await invoke("archive_bootstrap");
    renderLocations(state.locations);
  } catch (error) {
    // A failed refresh costs the counts, never the view: the operator asked
    // to change locations and must still land somewhere he can.
    showError(String(error));
  }
  elements.setupStatus.textContent = `${locations.length.toLocaleString()} folder${
    locations.length === 1 ? "" : "s"
  } chosen. Change them, or count what is there again.`;
}

function backToCensus() {
  hideError();
  if (lastReport) renderReport(lastReport);
  else showView("setup");
}

async function bootstrap() {
  try {
    const state = await invoke("archive_bootstrap");
    // Which build this is, in the footer, because "what does yours say?" is
    // where a support conversation starts. Two candidates once carried the
    // same version number and only one of them could index anything.
    if (state.buildIdentity) {
      elements.buildIdentity.textContent = `Minutes Archive ${state.buildIdentity}`;
    }
    renderLocations(state.locations);
    lastReport = state.report;
    vaultReport = state.textVaultReport;
    if (state.scanRunning) {
      showView("scanning");
    } else if (state.textVaultReport) {
      renderVaultSummary(state.textVaultReport);
      showView("search");
    } else if (state.report) {
      renderReport(state.report);
    } else {
      showView("setup");
    }
  } catch (error) {
    showError(error);
  }
}

elements.addLocations.addEventListener("click", chooseLocations);
elements.skipFolders.addEventListener("click", chooseSkippedFolders);
elements.clearSkipped.addEventListener("click", clearSkippedFolders);
elements.runCensus.addEventListener("click", runCensus);
elements.cancelCensus.addEventListener("click", () => cancelOperation(elements.cancelCensus));
elements.cancelIndexing.addEventListener("click", () => cancelOperation(elements.cancelIndexing));
elements.startOver.addEventListener("click", startOver);
elements.exportReport.addEventListener("click", exportReport);
elements.buildTextVault.addEventListener("click", buildTextVault);
elements.backToCensus.addEventListener("click", backToCensus);
elements.searchForm.addEventListener("submit", searchVault);
elements.dismissError.addEventListener("click", hideError);

bootstrap();
