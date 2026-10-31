// The words of Triage: they name things, states, consequences and next steps.

const proposalsTitle = 'Merge proposals';
const proposalsEmpty =
    'No proposals. Run Triage on a project to look for duplicates.';

String mergeConsequence(String keep, String dropped) =>
    "Merge: keeps '$keep', moves the content of '$dropped' into it and moves "
    "'$dropped' to trash. Undo is in page history.";

String mergedNote(String dropped) =>
    "'$dropped' is in the trash. Undo restores both pages.";

String triageStartedToast(String runner) =>
    'Triage started on $runner. Proposals appear under $proposalsTitle.';

String triageIntro(String project) =>
    "Triage reads the pages in '$project' and proposes merges for duplicate "
    'pages. Nothing changes until you merge a proposal.';

String triageFailed(Object e) => 'Triage did not start: $e';
String decisionFailed(String action, Object e) => '$action failed: $e';

const appliedTitle = 'Links and tags added by Triage';
String appliedLink(String from, String to) => "Linked '$from' to '$to'";
String appliedTag(String page, String tag) => "Tagged '$page' with $tag";
const appliedUndoNote =
    'Undo removes the added links and restores the previous tags of these pages. '
    'Tag changes made since are lost.';
