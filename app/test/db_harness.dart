// Database test fixture on the real core: a vault with the usual fixture
// pages plus a "Projects" database (every property type, six views, twelve
// rows with fixed dates in October 2026) and, on request, the seeded
// "Tasks" collection of the core (10,000 rows for the scrolling bench).
// The clock is fixed to October 6, 2026 so calendars and timelines are
// stable.

import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/debounce.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/addons/database/database_view.dart';
import 'package:notes_app/ui/database/values.dart';
import 'package:notes_app/shell/app_shell.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/shell/navigation.dart' as nav;
import 'package:notes_app/state/app_state.dart';
import 'package:notes_app/features/reminders/backends.dart';
import 'package:notes_app/features/settings/settings_store.dart';
import 'package:notes_app/state/collection_store.dart';
import 'package:notes_app/ui/ui.dart';

import 'harness.dart'
    show loadFonts, resetTestClock, finishAsync, createFixtureSpace;
import 'package:notes_app/core/clock.dart';

export 'harness.dart' show initCore, loadFonts;

/// The fixed "now" of database tests.
final fixedNow = DateTime(2026, 10, 6, 11, 20);

const projectsTitle = 'Projects';

/// Row titles of the Projects database, in order.
const projectRows = [
  'Website redesign',
  'Mobile app launch',
  'Quarterly report',
  'Customer interviews',
  'API migration',
  'Design system',
  'Onboarding flow',
  'Pricing page',
  'Data warehouse',
  'Team offsite',
  'Security audit',
  'Help center',
];

class DbVault {
  DbVault(this.dir, this.app, this.projects, this.tasks, this.host, this.schema);
  final Directory dir;
  final AppState app;

  /// The Projects database page.
  final String projects;

  /// The seeded Tasks collection (null unless asked for).
  final String? tasks;

  /// A plain page that holds an inline database of Projects.
  final String host;

  /// Projects' schema as saved (property name -> definition).
  final Map<String, PropDef> schema;

  Vault get vault => app.vault;

  String optionId(String prop, String name) => schema[prop]!.options.firstWhere((o) => o.name == name).id;
}

DbVault? _db;
DbVault? _readOnlySeed;

Future<void> disposeReadOnlySeed() async {
  final fixture = _readOnlySeed;
  _readOnlySeed = null;
  if (fixture != null) await fixture.dir.delete(recursive: true);
}
DbVault get db => _db!;

PropDef _def(String name, PropKind kind, [List<SelectOption> options = const [], String config = '']) =>
    PropDef(key: '', name: name, kind: kind, options: options, config: config);

SelectOption _opt(String name, String color) => SelectOption(id: '', name: name, color: color);

Future<String> _makeProjects(Vault vault) async {
  final id = await createFixtureSpace(vault, title: projectsTitle);
  await vault.run((v) => v.makeCollection(page: id), writes: true);
  final schema = await vault.run(
      (v) => v.setSchema(collection: id, schema: [
            _def('Status', PropKind.status, CollectionStore.defaultOptions(PropKind.status)),
            _def('Priority', PropKind.select, [_opt('High', 'red'), _opt('Medium', 'yellow'), _opt('Low', 'gray')]),
            _def('Tags', PropKind.multiSelect,
                [_opt('Design', 'purple'), _opt('Backend', 'blue'), _opt('Mobile', 'green'), _opt('Research', 'orange')]),
            _def('Due', PropKind.date),
            _def('Estimate', PropKind.number),
            _def('Approved', PropKind.checkbox),
            _def('Link', PropKind.url),
            _def('Owner email', PropKind.email),
            _def('Phone', PropKind.phone),
          ]),
      writes: true);
  final by = {for (final d in schema) d.name: d};
  String opt(String p, String n) => by[p]!.options.firstWhere((o) => o.name == n).id;
  final keys = [for (final d in schema) d.key];

  final saved = await vault.run((v) => v.listViews(collection: id));
  await vault.run((v) => v.updateView(collection: id, view: saved.first.copyWith(visibleProps: keys)), writes: true);
  ViewDef view(String name, ViewKind kind, {String? group, String? date, List<String>? props}) => ViewDef(
        id: '',
        name: name,
        kind: kind,
        filter: null,
        sorts: const [],
        groupBy: group,
        dateProp: date,
        visibleProps: props ?? keys,
        cardCover: null,
        config: '',
      );
  final status = by['Status']!.key, due = by['Due']!.key, tags = by['Tags']!.key, prio = by['Priority']!.key;
  final views = [
    view('Table', ViewKind.table),
    view('Board', ViewKind.board, group: status, props: [prio, tags, due]),
    view('Calendar', ViewKind.calendar, date: due, props: const []),
    view('Timeline', ViewKind.timeline, date: due, props: const []),
    view('List', ViewKind.list, props: [status, due]),
    view('Gallery', ViewKind.gallery, props: [status, tags]),
  ];
  // The page shows its views as view blocks. The saved views stay, as the
  // core keeps them for a page that got properties: old inline database
  // blocks still read them.
  for (final v in views.skip(1)) {
    await vault.run((x) => x.addView(collection: id, view: v), writes: true);
  }
  String? after;
  for (final (i, v) in views.indexed) {
    final block = 'pv${i + 1}';
    final prev = after;
    await vault.run((x) => x.insertViewBlock(pageId: id, id: block, after: prev, view: v), writes: true);
    after = block;
  }

  const statuses = ['Done', 'In progress', 'Not started'];
  const prios = ['High', 'Medium', 'Low'];
  const tagSets = [
    ['Design'], ['Mobile', 'Design'], ['Research'], ['Research', 'Design'], ['Backend'], ['Design'], //
    ['Mobile'], [], ['Backend', 'Research'], [], ['Backend'], ['Design', 'Research'],
  ];
  for (var i = 0; i < projectRows.length; i++) {
    final row = await vault.createPage(parent: id, title: projectRows[i]);
    final start = Day(2026, 10, 2 + i * 2);
    final values = <String, PropValue>{
      status: Values.status(opt('Status', statuses[i % 3])),
      prio: Values.select(opt('Priority', prios[(i * 2) % 3])),
      if (tagSets[i].isNotEmpty) tags: Values.multi([for (final t in tagSets[i]) opt('Tags', t)]),
      due: i % 4 == 1 ? Values.date(start.ms, end: start.add(3 + i % 3).ms) : Values.date(start.ms),
      by['Estimate']!.key: Values.number((i % 5 + 1) * 2.0),
      if (i % 3 == 0) by['Approved']!.key: Values.checkbox(true),
      if (i % 4 == 0) by['Link']!.key: Values.url('https://example.com/${projectRows[i].toLowerCase().replaceAll(' ', '-')}'),
      if (i % 5 == 0) by['Owner email']!.key: Values.text('team$i@example.com'),
      if (i % 6 == 0) by['Phone']!.key: Values.text('+1 555 010${i % 10}'),
    };
    for (final e in values.entries) {
      await vault.run((v) => v.setProperty(page: row, key: e.key, value: e.value), writes: true);
    }
  }
  return id;
}

Future<DbVault> _createVault({int tasks = 0, bool reuseReadOnlySeed = false}) async {
  resetTestClock();
  final cached = reuseReadOnlySeed ? _readOnlySeed : null;
  if (cached != null) {
    final vault = await Vault.open('${cached.dir.path}${Platform.pathSeparator}vault');
    final app = AppState(vault, reminderBackend: FakeReminderBackend());
    await app.start();
    return DbVault(cached.dir, app, cached.projects, cached.tasks, cached.host, cached.schema);
  }
  final dir = Directory.systemTemp.createTempSync('notes_db_test_');
  final root = '${dir.path}${Platform.pathSeparator}vault';
  String? tasksId;
  if (tasks > 0) {
    Directory(root).createSync(recursive: true);
    final seedWatch = Stopwatch()..start();
    tasksId = await NotesVault.seedTasks(root: root, rows: tasks);
    debugPrint('perf fixture seed: ${seedWatch.elapsedMilliseconds} ms ($tasks rows)');
  }
  final vault = await Vault.open(root);
  for (final t in ['Weekly review', 'Garden plan', 'Reading notes']) {
    final p = await createFixtureSpace(vault, title: t);
    await vault.apply(p, [
      BOp(op: OpKind.insert, id: newId(), kind: Kinds.paragraph, text: 'Some text about ${t.toLowerCase()}.', marks: const [], checked: false),
    ]);
  }
  final projects = await _makeProjects(vault);
  final host = await createFixtureSpace(vault, title: 'Planning');
  await vault.apply(host, [
    BOp(op: OpKind.insert, id: 'p1', kind: Kinds.paragraph, text: 'Projects for this quarter:', marks: const [], checked: false),
    BOp(op: OpKind.insert, id: 'db1', after: 'p1', kind: 'database', text: '', marks: const [], checked: false),
    BOp(op: OpKind.setTarget, id: 'db1', kind: 'database', text: '', marks: const [], checked: false, target: projects),
    BOp(op: OpKind.insert, id: 'p2', after: 'db1', kind: Kinds.paragraph, text: 'Notes after the database.', marks: const [], checked: false),
  ]);
  final schema = await vault.run((v) => v.getSchema(collection: projects));
  await vault.flush();
  final app = AppState(vault, reminderBackend: FakeReminderBackend());
  await app.start();
  final fixture = DbVault(dir, app, projects, tasksId, host, {for (final d in schema) d.name: d});
  if (reuseReadOnlySeed) _readOnlySeed = fixture;
  return fixture;
}

/// Pumps frames and lets bridge calls finish until the app is quiet.
Future<void> settle(WidgetTester tester, {Duration min = Duration.zero}) async {
  var quiet = 0;
  var elapsed = Duration.zero;
  const step = Duration(milliseconds: 50);
  final deadline = Stopwatch()..start();
  await tester.runAsync(() async { await _db?.app.feed.poll(); });
  while (deadline.elapsed < const Duration(seconds: 30)) {
    await tester.runAsync(() => Future<void>.delayed(Duration.zero));
    await tester.pump(step);
    elapsed += step;
    final feedIdle = _db?.app.feed.idle ?? true;
    final dataBusy = Vault.inFlight.value > 0 || !feedIdle || Debounce.pending > 0;
    final busy = dataBusy || tester.binding.hasScheduledFrame;
    quiet = busy ? 0 : quiet + 1;
    if (quiet >= 3 && elapsed >= min) return;
    // Keep the old 30s fake animation bound (spinners never become quiet),
    // but never return with native work pending: zero-delay yields can pump
    // 600 frames before a disk flush finishes under parallel load.
    if (!dataBusy && elapsed >= const Duration(seconds: 30)) return;
  }
  throw TestFailure('settle timed out: ${Vault.inFlight.value} bridge calls, '
      '${Debounce.pending} debounces pending');
}

/// Settles until [done] holds, up to [timeout] of real time.
Future<void> settleUntil(WidgetTester tester, bool Function() done, {Duration timeout = const Duration(seconds: 10)}) async {
  // Drain UI work, then force the core's writer through its existing ack.
  await settle(tester, min: Duration.zero);
  await tester.runAsync(() async { await _db?.vault.flush(); });
  final sw = Stopwatch()..start();
  while (!done() && sw.elapsed < timeout) {
    await tester.runAsync(() => Future<void>.delayed(Duration.zero));
    await settle(tester, min: Duration.zero);
  }
  // [done] may have turned true from a real timer after the last frame: let
  // the widgets catch up before returning.
  await settle(tester, min: Duration.zero);
}

Future<T> core<T>(WidgetTester tester, Future<T> Function(Vault v) f) async {
  final r = await tester.runAsync(() => f(db.vault));
  return r as T;
}

/// Rows of [collection] as the core has them now (default order).
Future<List<PageRow>> rowsOf(WidgetTester tester, String collection) async {
  await settle(tester);
  final r = await core(tester, (v) => v.run((x) => x.query(scope: collection, filter: null, sorts: const [], offset: 0, limit: 20000)));
  return r.rows;
}

Future<PageRow> rowNamed(WidgetTester tester, String title, [String? collection]) async =>
    (await rowsOf(tester, collection ?? db.projects)).firstWhere((r) => r.title == title);

/// Sets form factor, fonts, theme, size and the clock; makes the vault and
/// pumps the app on home.
Future<DbVault> pumpDb(WidgetTester tester, {required bool mobile, bool dark = false, int tasks = 0, bool reuseReadOnlySeed = false}) async {
  await tester.runAsync(loadFonts);
  sansFamily = 'Atkinson Hyperlegible Next';
  monoFamily = 'Fragment Mono';
  Debounce.pending = 0; // none can be left from an earlier test's zone
  viewBlockTabs.clear(); // active tabs are per session, keyed by block id
  // A steady caret: its blink phase would depend on how long settle pumped.
  EditableText.debugDeterministicCursor = true;
  debugDefaultTargetPlatformOverride = mobile ? TargetPlatform.android : TargetPlatform.windows;
  debugDisableShadows = false;
  tester.view.devicePixelRatio = mobile ? 2.0 : 1.0;
  tester.view.physicalSize = mobile ? const Size(375 * 2, 812 * 2) : const Size(1440, 900);
  tester.view.padding = mobile ? const FakeViewPadding(top: 24 * 2, bottom: 16 * 2) : FakeViewPadding.zero;
  tester.view.viewPadding = tester.view.padding;
  tester.view.viewInsets = FakeViewPadding.zero;
  tester.platformDispatcher.platformBrightnessTestValue = dark ? Brightness.dark : Brightness.light;
  addTearDown(() {
    tester.view.reset();
    tester.platformDispatcher.clearAllTestValues();
    EditableText.debugDeterministicCursor = false;
  });
  final v = (await tester.runAsync(() => _createVault(tasks: tasks, reuseReadOnlySeed: reuseReadOnlySeed)))!;
  _db = v;
  await tester.pumpWidget(
    MediaQuery.fromView(
      view: tester.view,
      child: Directionality(
        textDirection: TextDirection.ltr,
        child: NotesShell(
          navigatorKey: GlobalKey<NavigatorState>(),
          wrap: (child) => SettingsScope(settings: v.app.settings, child: AppScope(state: v.app, child: child)),
          home: const HomeScreen(),
        ),
      ),
    ),
  );
  await settle(tester);
  return v;
}

Future<void> _close(WidgetTester tester) async {
  final v = _db;
  if (v == null) return;
  await tester.pumpWidget(const SizedBox.shrink());
  await settle(tester);
  await finishAsync(tester, v.app.dispose());
  await tester.runAsync(() async {
    try {
      if (v.dir != _readOnlySeed?.dir) await v.dir.delete(recursive: true);
    } catch (_) {}
  });
  _db = null;
  await tester.pump(const Duration(seconds: 3));
}

/// testWidgets with the database fixture's set-up and tear-down.
void dbTest(String description, Future<void> Function(WidgetTester tester) body, {bool skip = false, Timeout? timeout}) {
  testWidgets(description, (tester) async {
    try {
      await body(tester);
    } finally {
      await _close(tester);
      debugDefaultTargetPlatformOverride = null;
      debugDisableShadows = true;
      Clock.fixedAt = null;
    }
  }, skip: skip, timeout: timeout);
}

// ---- navigation and finders ----

Finder byKey(String k) => find.byKey(ValueKey(k));

/// Pops the sheets above the first page (a row opened on top of its
/// database).
Future<void> closeRowSheet(WidgetTester tester) async {
  final stack = nav.PageStackScope.of(tester.element(find.byType(HomeScreen)));
  if (stack.pages.length > 1) stack.popTo(stack.pages.first);
  await settle(tester);
}

/// Opens the page titled [title]: from the sidebar on desktop; on mobile
/// through the app's navigation (home lists pages by recency, and the
/// database rows push the fixture pages below the fold).
Future<void> openPage(WidgetTester tester, String title) async {
  final pages = await core(tester, (v) => v.listChildren(null));
  final id = pages.firstWhere((p) => p.title == title).id;
  // Agenda source labels can share this title; target the actual page row.
  final tile = byKey('tree-$id').hitTestable();
  if (tile.evaluate().isNotEmpty && tester.view.devicePixelRatio == 1.0) {
    await tester.tap(tile.last);
  } else {
    nav.openPage(tester.element(find.byType(HomeScreen)), id, title);
  }
  await settle(tester);
}

Future<void> tapKey(WidgetTester tester, String k) async {
  final f = byKey(k).last;
  if (!_hits(tester, f)) {
    // Centred, so that frozen columns and toolbars do not cover it.
    await Scrollable.ensureVisible(tester.element(f), alignment: 0.5);
    await settle(tester);
  }
  await tester.tap(f);
  await settle(tester);
}

/// Whether a tap on the centre of [f] reaches it (on screen, not covered).
bool _hits(WidgetTester tester, Finder f) {
  final c = tester.getCenter(f, warnIfMissed: false);
  if (!tester.view.physicalSize.contains(c * tester.view.devicePixelRatio)) return false;
  final box = tester.renderObject(f);
  return tester.hitTestOnBinding(c).path.any((e) => e.target == box);
}

/// Types [text] into the text field under the widget keyed [k].
Future<void> typeInto(WidgetTester tester, String k, String text) async {
  final f = byKey(k).last;
  final field = find.descendant(of: f, matching: find.byType(EditableText), matchRoot: true);
  await tester.enterText(field.first, text);
  await settle(tester);
}

/// Submits the focused text field (the Enter key of a soft keyboard).
Future<void> submit(WidgetTester tester) async {
  await tester.testTextInput.receiveAction(TextInputAction.done);
  await settle(tester);
}

/// Closes the top popover or sheet by tapping its barrier.
Future<void> dismiss(WidgetTester tester) async {
  final size = tester.view.physicalSize / tester.view.devicePixelRatio;
  await tester.tapAt(Offset(size.width - 4, tester.view.devicePixelRatio == 1.0 ? size.height - 4 : 30));
  await settle(tester);
}

/// Opens Projects and shows the view tab [view].
Future<void> openView(WidgetTester tester, String view, {bool mobile = false}) async {
  await openPage(tester, projectsTitle);
  if (view == 'Table') return;
  if (mobile) {
    await tapKey(tester, 'view-switcher');
    await tapKey(tester, 'sheet-view-$view');
    if (byKey('views-done').evaluate().isNotEmpty) await tapKey(tester, 'views-done');
  } else {
    // Tabs past the text column's width sit under "N more…".
    if (byKey('tab-$view').evaluate().isEmpty) {
      await tapKey(tester, 'tabs-more');
      await tapKey(tester, 'more-view-$view');
    } else {
      await tapKey(tester, 'tab-$view');
    }
  }
}

/// The settings of [page]'s view blocks as the core has them, in order.
Future<List<ViewDef>> viewBlocksOf(WidgetTester tester, String page) async {
  await settle(tester);
  return core(tester, (v) async {
    final p = await v.run((x) => x.loadPage(id: page));
    return [
      for (final b in p.blocks)
        if (b.kind == 'view') await v.run((x) => x.viewBlockDef(id: b.id, text: b.text)),
    ];
  });
}

DatabaseViewState dbState(WidgetTester tester) => tester.state<DatabaseViewState>(find.byType(DatabaseView).first);

/// Moves a mouse over [f] (desktop hover effects).
Future<TestGesture> hover(WidgetTester tester, Finder f) async {
  final g = await tester.createGesture(kind: PointerDeviceKind.mouse);
  await g.addPointer(location: Offset.zero);
  await g.moveTo(tester.getCenter(f));
  await settle(tester);
  return g;
}

/// Drags [from] to the global point [to] with a mouse, in steps.
/// A wide table's content centre can lie outside its clipped viewport.
Offset visibleCenter(WidgetTester tester, Finder finder) {
  final screen =
      Offset.zero & (tester.view.physicalSize / tester.view.devicePixelRatio);
  final visible = tester.getRect(finder).intersect(screen);
  expect(visible.isEmpty, isFalse, reason: 'gesture target must be visible');
  return visible.center;
}

Future<void> dragTo(WidgetTester tester, Finder from, Offset to, {bool longPress = false}) async {
  final start = tester.getCenter(from);
  final g = await tester.startGesture(start, kind: longPress ? PointerDeviceKind.touch : PointerDeviceKind.mouse);
  if (longPress) {
    await tester.pump(const Duration(milliseconds: 600));
  }
  await tester.pump(const Duration(milliseconds: 50));
  const steps = 12;
  for (var i = 1; i <= steps; i++) {
    await g.moveTo(Offset.lerp(start, to, i / steps)!);
    await tester.pump(const Duration(milliseconds: 16));
  }
  await g.up();
  await settle(tester);
}

/// Sends a key press.
Future<void> press(WidgetTester tester, LogicalKeyboardKey k) async {
  await tester.sendKeyEvent(k);
  await settle(tester);
}

Future<void> shot(String name) async {

  await expectLater(find.byType(Directionality).first, matchesGoldenFile('../screenshots/$name.png'));

}
