// Copy for conflict review: the page toplines, the inline block review, the
// toasts and the settings rows. Strings marked provisional are not in the
// approved merge mockup (design/merge/index.html); review them before release.
//
// Where a side has no local version (both sides are other devices), the copy
// names the devices instead of "the cloud version" or "the local version". The
// function strings below take those names; they are provisional too.

// ---- toplines (subtree and page) ----

/// The cloud edited a section this device deleted; the section is back.
const sectionBackCloudCopy = "The cloud version edited this section after you deleted it here, so it's back.";

/// This device's version edited a section another device deleted; it is kept. // provisional
const sectionBackLocalCopy = "The local version edited this section after it was deleted on another device, so it's kept.";

/// The cloud changed a page this device trashed; the core took it out of Trash.
const pageTrashedCopy = "The cloud version changed this page after you deleted it here, so it's back in Pages.";

/// This device's version changed a page another device trashed; it is kept. // provisional
const pageTrashedLocalCopy = "The local version changed this page after it was deleted on another device, so it's back in Pages.";

/// The cloud changed a page this device permanently deleted.
const pagePurgedCopy = "The cloud version changed this page after it was permanently deleted here, so it's back in Pages.";

/// This device's version changed a page another device permanently deleted; it is kept. // provisional
const pagePurgedLocalCopy = "The local version changed this page after it was permanently deleted on another device, so it's back in Pages.";

/// No local side: [editor] edited a section that [deleter] deleted; it is back. // provisional
String sectionBackNamedCopy(String editor, String deleter) => "$editor edited this section after $deleter deleted it, so it's back.";

/// No local side: [editor] changed a page that [deleter] trashed; it came back. // provisional
String pageTrashedNamedCopy(String editor, String deleter) => "$editor changed this page after $deleter deleted it, so it's back in Pages.";

/// No local side: [editor] changed a page that [deleter] permanently deleted. // provisional
String pagePurgedNamedCopy(String editor, String deleter) => "$editor changed this page after $deleter permanently deleted it, so it's back in Pages.";

const conflictOk = 'OK';
const conflictDeleteAgain = 'Delete again';
const conflictUseLocal = 'Use the local version';
const conflictUseCloud = 'Use the cloud version';

/// No local side: the button takes the version from [device]. // provisional
String conflictUseDevice(String device) => 'Use the version from $device';

// ---- inline block review ----

const conflictLocalLabel = 'Local';

/// The other device's version of a block changed on both devices.
String conflictCloudLabel(String device) => 'Cloud · from $device';

/// Shown in place of a block whose text is empty. // provisional
const conflictEmptyBlock = 'Empty block';

// ---- toasts ----

const conflictUsedCloudLine = 'Used the cloud line.';

/// The note showed the cloud's line and the local one was tapped. // provisional
const conflictUsedLocalLine = 'Used the local line.';

/// No local side: the line from [device] was used. // provisional
String conflictUsedDeviceLine(String device) => 'Used the line from $device.';

const conflictDeletedAgain = 'Deleted again.';
const conflictUndo = 'Undo';

/// A section was removed by choosing the deleted version.
String conflictSectionRemoved(String heading) => heading.isEmpty ? 'Section removed.' : '$heading section removed.';

// ---- summaries and settings ----

const conflictSameLine = 'Same line edited on both';
const conflictBroughtFromTrash = 'Cloud brought it back from Trash';

/// The cloud restored a section. // provisional wording for the summary's heading fallback
String conflictCloudBroughtBack(String heading) =>
    'Cloud brought ${heading.isEmpty ? 'this section' : heading} back';

/// This device restored a section another device deleted. // provisional
String conflictLocalBroughtBack(String heading) =>
    'This device brought ${heading.isEmpty ? 'this section' : heading} back';

/// No local side: [device] restored a section. // provisional
String conflictDeviceBroughtBack(String device, String heading) =>
    '$device brought ${heading.isEmpty ? 'this section' : heading} back';

/// The cloud brought a page back after it was permanently deleted. // provisional
const conflictCloudBroughtBackPurged = 'Cloud brought it back';

/// This device brought a page back after another device permanently deleted it. // provisional
const conflictLocalBroughtBackPurged = 'This device brought it back';

/// This device brought a page back from Trash. // provisional
const conflictLocalBroughtFromTrash = 'This device brought it back from Trash';

/// No local side: [device] brought a page back. // provisional
String conflictDeviceBroughtBackPurged(String device) => '$device brought it back';

/// No local side: [device] brought a page back from Trash. // provisional
String conflictDeviceBroughtFromTrash(String device) => '$device brought it back from Trash';

const conflictsSectionTitle = 'Changed on two devices';

/// Settings page title, and its intro.
const conflictsSettingsTitle = 'Conflicts';
const conflictsIntro =
    'When the same page changes on two devices before they sync, Scratch keeps both versions and lists the page here.';

/// Settings empty state.
const conflictsNoneTitle = 'Nothing to review';
const conflictsNone = 'No page has changed on two devices at once.';

const conflictAutoResolveName = 'Resolve simple conflicts on their own';
const conflictAutoResolveDesc =
    'On: a change that can settle itself leaves this list. Off: it waits here until you confirm it.';
const conflictAutoResolveHelp = 'Turn off if you want to see every conflict, even ones Scratch can settle.';

/// The auto-resolve switch could not be saved; it goes back to its old value. // provisional
const conflictAutoResolveSaveFailed = 'Could not change this setting. Try again.';

// ---- review log (Settings > Conflicts, Recent) ----

/// The log's heading. // provisional
const conflictsRecentTitle = 'Resolved in the last 30 days';

/// The log is empty. // provisional
const conflictsRecentNone = 'Nothing resolved in the last 30 days.';

/// An old application did not record the chosen resolution.
const conflictLogResolved = 'Resolved';

const conflictLogKept = 'Kept';
const conflictLogUsedLocal = 'Used the local version';
const conflictLogUsedCloud = 'Used the cloud version';

/// An automatic rule applied the version. // provisional
const conflictLogAuto = 'Settled on its own';

/// The latest choice was undone; the entry is back in the list. // provisional
const conflictLogUndone = 'Undone';

/// One log row's detail: what happened and when. // provisional
String conflictLogDetail(String what, String when) => '$what · $when';
