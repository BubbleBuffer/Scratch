// Shared fixture for the kanban add-on tests. The stored shape comes from the
// approved design (wiki/addons/kanban.md); everything visible is reached
// through the real widgets. Kit keys and labels (board renderer, block
// pickers, embed) are used as they are. The few kanban-owned labels live in
// the constants below so one edit covers them.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/registry/command_registry.dart';
import 'package:notes_app/services/addon_services.dart' show openPage;
import 'package:notes_app/ui/board/board_renderer.dart';
import 'package:notes_app/ui/menu.dart';

import '../../addon_harness.dart';

export '../../addon_harness.dart';

/// The page type and the add-on id under which the kanban keeps its data.
const kanbanType = 'kanban';
const kanbanAddonId = 'kanban';

/// Starter columns of a new board, in order.
const starterColumns = ['To do', 'Doing', 'Done'];

// Kanban-owned strings and keys, as lib/addons/kanban/ defines them.
const newKanbanLabel = 'New kanban';
const addExistingLabel = 'Add existing note…';
const removeFromBoardLabel = 'Remove from board';
const removeCardKey = ValueKey('kanban-remove-card');
const slashKanbanKey = ValueKey('slash-kanban');
const cardTitleFieldKey = ValueKey('kanban-card-title');

// Kit strings (board renderer), known from lib/ui/board/board_renderer.dart.
const deleteColumnLabel = 'Delete column';

/// Kit key of the add-card button at the foot of [column] (its keyLabel is
/// the column title).
String addButtonKey(String column) => 'board-new-$column';

/// The kanban's stored data: its columns, in order, and the cards of each
/// column as page ids.
Future<Map<String, dynamic>> storedKanban(
  WidgetTester tester,
  String page,
) async {
  final raw = await core(tester, (v) => v.typeData(page, kanbanAddonId));
  return jsonDecode(raw) as Map<String, dynamic>;
}

List<String> storedTitles(Map<String, dynamic> data) {
  final out = <String>[];
  for (final c in data['columns'] as List<dynamic>) {
    out.add((c as Map<String, dynamic>)['title'] as String);
  }
  return out;
}

String columnIdOf(Map<String, dynamic> data, String title) {
  for (final c in data['columns'] as List<dynamic>) {
    final column = c as Map<String, dynamic>;
    if (column['title'] == title) return column['id'] as String;
  }
  throw TestFailure('No column "$title" in the stored board');
}

List<String> storedCards(Map<String, dynamic> data, String columnTitle) {
  final id = columnIdOf(data, columnTitle);
  final cards = (data['cards'] as Map<String, dynamic>?) ?? const {};
  final ids = (cards[id] as List<dynamic>?) ?? const [];
  return [for (final c in ids) c as String];
}

/// Creates a kanban page the way the add-on stores one: columns and cards in
/// its type data. [columns] maps each title to the note ids in it, in order.
/// Tests use this only for set-up; the behaviour under test goes through the
/// widgets.
Future<String> seedKanban(
  WidgetTester tester, {
  required String title,
  required Map<String, List<String>> columns,
  String? parent,
}) async {
  final cols = <Map<String, String>>[];
  final cards = <String, List<String>>{};
  var n = 0;
  for (final entry in columns.entries) {
    final id = 'col${++n}';
    cols.add({'id': id, 'title': entry.key});
    cards[id] = entry.value;
  }
  return core(tester, (v) async {
    final page = await v.createPage(
      parent: parent,
      title: title,
      pageType: kanbanType,
    );
    await v.typeDataSet(page, kanbanAddonId, const [
      'columns',
    ], jsonEncode(cols));
    await v.typeDataSet(page, kanbanAddonId, const [
      'cards',
    ], jsonEncode(cards));
    return page;
  });
}

/// Opens a page in the app's stack, as a tap would, and lets it settle.
Future<void> openNote(WidgetTester tester, String id, String title) async {
  unawaited(openPage(tester.element(find.byType(Overlay).first), id, title));
  await settle(tester);
}

// ---------------------------------------------------------------------------
// Finders (kit keys: board-col-<title>, board-card-<id> on each card surface,
// drop-<id>, group-menu-<title>, board-column-left/right-<id>). A card's body
// is drawn by its note type, so a card is found by the text of its title
// inside its surface, not by a key on the body.

Finder columnNamed(String title) => find.byKey(ValueKey('board-col-$title'));

/// Every card surface on the board, whatever the column or note.
Finder allCardSurfaces() => find.byWidgetPredicate(
  (w) =>
      w.key is ValueKey &&
      '${(w.key as ValueKey).value}'.startsWith('board-card-'),
  description: 'card surface',
);

/// The card showing [title]: its title text inside a card surface.
Finder cardNamed(String title) =>
    find.descendant(of: allCardSurfaces(), matching: find.text(title));

Finder cardIn(String column, String title) =>
    find.descendant(of: columnNamed(column), matching: cardNamed(title));

/// Every card surface in [column], whatever its title (a missing note has no
/// readable title, so it is counted by id).
Finder cardSurfacesIn(String column) =>
    find.descendant(of: columnNamed(column), matching: allCardSurfaces());

/// The kit board drawn inline (the embed's own renderer), inside [host].
Finder inlineBoardIn(Finder host) => find.descendant(
  of: host,
  matching: find.byWidgetPredicate(
    (w) => w is BoardRenderer && w.inline,
    description: 'inline board renderer',
  ),
);

/// Asserts the board has loaded: each column in [cards] is on screen and shows
/// each of its notes' titles. Goldens call this before every screenshot, so a
/// shot never captures a board that is still empty or loading.
void expectBoardLoaded(Map<String, List<String>> cards) {
  for (final entry in cards.entries) {
    expect(
      columnNamed(entry.key),
      findsOneWidget,
      reason: 'column ${entry.key}',
    );
    for (final title in entry.value) {
      expect(
        cardNamed(title),
        findsOneWidget,
        reason: 'card $title in ${entry.key}',
      );
    }
  }
}

Finder columnMenu(String title) => find.byKey(ValueKey('group-menu-$title'));

Finder addButton(String column) => find.byKey(ValueKey(addButtonKey(column)));

/// The titles of the columns on screen, left to right.
List<String> columnsOnScreen(WidgetTester tester) {
  final found = <(double, String)>[];
  for (final title in [...starterColumns, 'Ideas', 'Review', 'Backlog']) {
    final f = columnNamed(title);
    if (f.evaluate().isEmpty) continue;
    found.add((tester.getTopLeft(f).dx, title));
  }
  found.sort((a, b) => a.$1.compareTo(b.$1));
  return [for (final f in found) f.$2];
}

// ---------------------------------------------------------------------------
// Gestures, copied in spirit from test/board_renderer_test.dart.

/// Drags the card titled [title] to [destination]; touch waits out the long
/// press that starts a mobile drag.
Future<void> dragCard(
  WidgetTester tester,
  String title,
  Offset destination, {
  bool mobile = false,
}) async {
  final start = tester.getCenter(cardNamed(title));
  final gesture = await tester.startGesture(
    start,
    kind: mobile ? PointerDeviceKind.touch : PointerDeviceKind.mouse,
  );
  if (mobile) await tester.pump(const Duration(milliseconds: 600));
  for (var i = 1; i <= 12; i++) {
    await gesture.moveTo(Offset.lerp(start, destination, i / 12)!);
    await tester.pump(const Duration(milliseconds: 16));
  }
  await gesture.up();
  await settle(tester);
}

/// Scrolls the board until [f] is on screen (a phone shows about one column),
/// then taps it.
Future<void> revealAndTap(WidgetTester tester, Finder f) async {
  await tester.ensureVisible(f);
  await tapAndSettle(tester, f);
}

/// Opens the context menu of the card titled [title] (a secondary click on
/// desktop).
Future<void> openCardMenu(WidgetTester tester, String title) async {
  await tester.tap(cardNamed(title), buttons: kSecondaryButton);
  await settle(tester);
}

/// Types [title] into the card title field that the New card flow shows, and
/// submits it.
Future<void> typeCardTitle(WidgetTester tester, String title) async {
  await tester.enterText(
    find.descendant(
      of: find.byKey(cardTitleFieldKey),
      matching: find.byType(EditableText),
    ),
    title,
  );
  await tester.testTextInput.receiveAction(TextInputAction.done);
  await settle(tester);
}

// ---------------------------------------------------------------------------
// Card ellipsis. The trigger key is the one assumption here: it is keyed by the
// card's note id, and the source worker's key must match this one. Every use
// goes through cardMenuTrigger so one edit covers a change.

const moveToColumnLabel = 'Move to column…';

/// The ellipsis of the card for note [cardId] (desktop: shown on hover; phone:
/// always shown).
Finder cardMenuTrigger(String cardId) =>
    find.byKey(ValueKey('kanban-card-menu-$cardId'));

/// A row of the open card menu, by its label.
Finder menuItem(String label) => find.widgetWithText(MenuItem, label);

/// Moves the desktop pointer over the card titled [title] and lets the hover
/// state settle. The pointer is removed when the test ends.
Future<void> hoverCard(WidgetTester tester, String title) async {
  final pointer = await tester.createGesture(kind: PointerDeviceKind.mouse);
  addTearDown(pointer.removePointer);
  await pointer.addPointer(location: Offset.zero);
  await pointer.moveTo(tester.getCenter(cardNamed(title)));
  await settle(tester);
}

/// The opacity of the card's ellipsis: 0 while it is hidden (desktop, pointer
/// off the card) and 1 while it is shown. The button is always in the tree,
/// so visibility is read from the Opacity that wraps it, not from its presence.
double menuOpacity(WidgetTester tester, String cardId) => tester
    .widget<Opacity>(
      find
          .ancestor(of: cardMenuTrigger(cardId), matching: find.byType(Opacity))
          .first,
    )
    .opacity;

const moveCardKey = ValueKey('kanban-move-card');

/// The phone's Move to column… target row for [columnId] (its label is the
/// column's title).
Finder moveTarget(String columnId) =>
    find.byKey(ValueKey('kanban-move-target-$columnId'));

// ---------------------------------------------------------------------------
// Readable Markdown. Lines follow the source's projection (markdown.dart): a
// heading per column, `## <title>`, then `- [[<id>|<title>]]` per card, or
// `- [[<id>]]` when the title equals the id.

/// Adds the existing note [id] (titled [title]) to [column] through the board's
/// add-existing picker.
Future<void> addExisting(
  WidgetTester tester,
  String column,
  String id,
  String title,
) async {
  await tapAndSettle(tester, addButton(column));
  await tapAndSettle(tester, find.text(addExistingLabel));
  await tester.enterText(
    find.descendant(
      of: find.byKey(const ValueKey('page-picker-field')),
      matching: find.byType(EditableText),
    ),
    title,
  );
  await settle(tester);
  await tapAndSettle(tester, find.byKey(ValueKey('page-pick-$id')));
  await settleUntil(tester, () => cardIn(column, title).evaluate().isNotEmpty);
}

String headingLine(String title) => '## $title';

String cardLinkLine(String id, String title) =>
    title == id ? '- [[$id]]' : '- [[$id|$title]]';

/// The heading and page-link lines of a readable body, in order.
List<String> outlineOf(String body) => [
  for (final line in const LineSplitter().convert(body))
    if (line.startsWith('## ') || RegExp(r'^- \[\[[^\]]+\]\]$').hasMatch(line))
      line,
];

/// The readable body that follows the front matter of an exported page.
String bodyOf(String exported) {
  expect(exported.startsWith('---\n'), isTrue, reason: 'front matter first');
  final rest = exported.substring(4);
  final end = rest.indexOf('\n---\n');
  expect(end, greaterThanOrEqualTo(0), reason: 'front matter ends');
  return rest.substring(end + 5);
}

/// The JSON of an exported page's `type-data:` line.
Map<String, dynamic> typeDataOf(String exported) {
  final line = exported
      .split('\n')
      .firstWhere((l) => l.startsWith('type-data: '));
  return jsonDecode(line.substring('type-data: '.length))
      as Map<String, dynamic>;
}

/// The Markdown file the vault wrote for [id] (its name ends with `-<id>.md`).
Future<File> pageFileOf(WidgetTester tester, String id) async {
  final pages = Directory(
    '${current.dir.path}${Platform.pathSeparator}vault${Platform.pathSeparator}pages',
  );
  return (await tester.runAsync(
    () async => pages.listSync().whereType<File>().singleWhere(
      (f) => f.path.endsWith('-$id.md'),
    ),
  ))!;
}

/// Creates a board with the real New kanban command and opens nothing else:
/// the command opens it. Returns the board's id.
Future<String> newKanbanBoard(WidgetTester tester) async {
  final command = commandRegistry.all.singleWhere(
    (c) => c.label == newKanbanLabel,
  );
  commandRegistry.run(command.id, tester.element(find.byType(Overlay).first));
  await settleUntil(tester, () => columnNamed('Doing').evaluate().isNotEmpty);
  final top = await core(tester, (v) => v.listChildren(null));
  return top.singleWhere((p) => p.pageType == kanbanType).id;
}

/// The one child page of [parent] (the board's only card at that step).
Future<String> onlyChildOf(WidgetTester tester, String parent) async {
  final children = await core(tester, (v) => v.listChildren(parent));
  expect(children, hasLength(1));
  return children.single.id;
}

/// Checks that the stored readable body of [board] lists exactly [expected].
/// The body is written after the board change, so this waits for it first.
Future<void> expectOutline(
  WidgetTester tester,
  String board,
  List<String> expected,
) async {
  for (var i = 0; i < 40; i++) {
    final body = await core(tester, (v) => v.typeMarkdownBody(board));
    if (body != null && outlineOf(body).join('\n') == expected.join('\n')) {
      return;
    }
    await settle(tester, min: Duration.zero);
  }
  final body = await core(tester, (v) => v.typeMarkdownBody(board));
  expect(body, isNotNull, reason: 'the board has a readable body');
  expect(outlineOf(body!), expected);
}

/// The board's keyboard stop for the card at [index] in reading order (the
/// first card by default). A card is one focus stop; its body is excluded.
FocusNode cardFocus(WidgetTester tester, [int index = 0]) => tester
    .widget<FocusableActionDetector>(
      find
          .descendant(
            of: allCardSurfaces().at(index),
            matching: find.byType(FocusableActionDetector),
          )
          .first,
    )
    .focusNode!;

/// True when the keyboard focus is on, or inside, a widget found by [finder].
bool focusIsIn(Finder finder) {
  final context = FocusManager.instance.primaryFocus?.context;
  if (context == null) return false;
  final targets = finder.evaluate().toSet();
  if (targets.contains(context)) return true;
  var found = false;
  context.visitAncestorElements((e) {
    found = targets.contains(e);
    return !found;
  });
  return found;
}
