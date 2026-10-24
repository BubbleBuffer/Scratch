// Test harness against the REAL core: the bridge library built for the host
// (`cargo build -p notes_bridge --release`) is loaded once, and every test
// gets its own vault in a temporary folder. Also bundled Stacks fonts,
// form factor and theme switches, and a stand-in
// keyboard for mobile screenshots.
//
// Bridge calls complete on the real event loop, so tests wait with
// [settle] (short runAsync windows between pumps) instead of pumpAndSettle.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/debounce.dart';
import 'package:notes_app/core/clock.dart';
import 'package:notes_app/src/rust/api/clock.dart' as rust_clock;
import 'package:notes_app/core/file_dialogs.dart';
import 'package:notes_app/core/link_previews.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/editor/rich_controller.dart';
import 'package:notes_app/features/showcase.dart';
import 'package:notes_app/features/demo.dart';
import 'package:notes_app/features/getting_started.dart';
import 'package:notes_app/shell/addons.dart';
import 'package:notes_app/shell/app_shell.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/src/rust/frb_generated.dart';
import 'package:notes_app/state/app_state.dart';
import 'package:notes_app/features/reminders/backends.dart';
import 'package:notes_app/features/settings/settings_store.dart';
import 'package:notes_app/state/ui_state.dart';
import 'package:notes_app/addons/kanban/api.dart' show RunnerAssignments, RunnerScope;
import 'package:notes_app/ui/ui.dart';

import 'fake_dialogs.dart';

export 'fake_dialogs.dart';

/// The host build of the bridge (Windows).
String bridgeLibrary() {
  final forced = Platform.environment['NOTES_BRIDGE_LIB'];
  if (forced != null && forced.isNotEmpty) return forced;
  // `flutter test` runs in app/; the cargo workspace target is one level up.
  final root = Directory.current.parent.path;
  return '$root${Platform.pathSeparator}target${Platform.pathSeparator}release${Platform.pathSeparator}${Platform.isWindows
      ? 'notes_bridge.dll'
      : Platform.isMacOS
      ? 'libnotes_bridge.dylib'
      : 'libnotes_bridge.so'}';
}

bool _rustReady = false;

// Opt-in diagnostics keep the bridge's original future and handler behavior.
// Record names/stacks only; arguments may contain pairing credentials.
final _bridgeTrace = Platform.environment['NOTES_TRACE_BRIDGE_CALLS'] == '1'
    ? _TracingBridgeHandler()
    : null;

class _PendingBridgeCall {
  _PendingBridgeCall(this.name)
    : elapsed = Stopwatch()..start(),
      startedAt = StackTrace.current;

  final String name;
  final Stopwatch elapsed;
  final StackTrace startedAt;
}

class _TracingBridgeHandler extends BaseHandler {
  final _pending = <Object, _PendingBridgeCall>{};

  @override
  Future<S> executeNormal<S, E extends Object>(NormalTask<S, E> task) {
    final id = Object();
    _pending[id] = _PendingBridgeCall(task.constMeta.debugName);
    final Future<S> future;
    try {
      future = super.executeNormal(task);
    } catch (_) {
      _pending.remove(id);
      rethrow;
    }
    // Observe completion without replacing the future returned to callers,
    // adding timers, or changing error propagation on that original future.
    unawaited(
      future.then<void>(
        (_) {
          _pending.remove(id);
        },
        onError: (Object _, StackTrace _) {
          _pending.remove(id);
        },
      ),
    );
    return future;
  }

  String pendingReport() {
    if (_pending.isEmpty) return '\nPending native calls: none recorded';
    return '\nPending native calls:\n${_pending.values.map((call) {
      final stack = call.startedAt.toString().split('\n').take(14).join('\n');
      return '${call.name} (${call.elapsed.elapsedMilliseconds} ms)\n$stack';
    }).join('\n')}';
  }
}

final fixedNow = DateTime(2026, 10, 6, 11, 20);

/// Reset the app clock before opening a test vault. The core clock runs
/// monotonically for the whole file, so imported edits also sort after seeds.
void resetTestClock() {
  Clock.fixedAt = fixedNow;
  Debounce.instant = true;
}

/// Loads the bridge and registers the built-ins; call from setUpAll.
Future<void> initCore() async {
  if (_rustReady) return;
  _rustReady = true;
  await RustLib.init(
    handler: _bridgeTrace,
    externalLibrary: ExternalLibrary.open(bridgeLibrary()),
  );
  rust_clock.setTestClock(startMs: fixedNow.millisecondsSinceEpoch);
  registerBuiltins();
  FileDialogs.instance = FakeFileDialogs();
}

/// What bookmark previews "fetch" in tests, by URL (no network).
final Map<String, LinkPreview> fakeLinks = {};

/// URLs the bookmark previews asked for.
final List<String> fetchedLinks = [];

LinkPreviews _links() => LinkPreviews(
  fetch: (url) async {
    fetchedLinks.add(url);
    return fakeLinks[url];
  },
);

/// The file dialogs of the running test ([pumpApp] installs fresh ones).
FakeFileDialogs get dialogs => FileDialogs.instance as FakeFileDialogs;

bool _fontsLoaded = false;

Future<void> _load(
  String family,
  List<String> paths, {
  bool required = false,
}) async {
  final loader = FontLoader(family);
  var any = false;
  for (final p in paths) {
    final f = File(p);
    if (!f.existsSync()) {
      if (required) throw TestFailure('Required bundled font missing: $p');
      continue;
    }
    final bytes = f.readAsBytesSync();
    loader.addFont(Future.value(ByteData.sublistView(bytes)));
    any = true;
  }
  if (any) await loader.load();
}

Future<void> loadFonts() async {
  if (_fontsLoaded) return;
  final win = '${Platform.environment['WINDIR'] ?? r'C:\Windows'}\\Fonts';
  await _load('Atkinson Hyperlegible Next', [
    'assets/fonts/AtkinsonHyperlegibleNext[wght].ttf',
    'assets/fonts/AtkinsonHyperlegibleNext-Italic[wght].ttf',
  ], required: true);
  await _load('Bricolage Grotesque', [
    'assets/fonts/BricolageGrotesque[opsz,wdth,wght].ttf',
  ], required: true);
  await _load('Fragment Mono', [
    'assets/fonts/FragmentMono-Regular.ttf',
    'assets/fonts/FragmentMono-Italic.ttf',
  ], required: true);
  await _load('Segoe UI Emoji', ['$win\\seguiemj.ttf']);
  // The Lucide icon fonts (package fonts register as packages/<pkg>/<family>).
  final lucide = _packageRoot('lucide_icons_flutter');
  await _load('packages/lucide_icons_flutter/Lucide', [
    lucide.resolve('assets/lucide.ttf').toFilePath(),
  ], required: true);
  for (final w in ['300', '500']) {
    await _load('packages/lucide_icons_flutter/Lucide$w', [
      lucide.resolve('assets/build_font/LucideVariable-w$w.ttf').toFilePath(),
    ], required: true);
  }
  _fontsLoaded = true;
}

/// The root folder of package [name] (ending in /), from
/// .dart_tool/package_config.json.
Uri _packageRoot(String name) {
  final config = File('.dart_tool/package_config.json').absolute;
  final json = jsonDecode(config.readAsStringSync()) as Map<String, dynamic>;
  for (final p in (json['packages'] as List).cast<Map<String, dynamic>>()) {
    if (p['name'] == name) {
      final root = p['rootUri'] as String;
      return config.uri.resolve(root.endsWith('/') ? root : '$root/');
    }
  }
  throw TestFailure('Package $name is not in package_config.json');
}

// ---------------------------------------------------------------------------
// Vault fixture

/// Page titles the fixture creates, oldest first (the showcase is created
/// last, so it is the most recently edited page).
const fixtureTitles = [
  'Weekly review',
  'Garden plan',
  'Recipes',
  'Travel checklist',
  'Reading notes',
  'Budget',
];

/// Subpages created under "Recipes".
const recipeChildren = ['Soups', 'Breads'];

/// The colours a top-level page used to get from the automatic cycle, in
/// order. Fixtures that showcase coloured spaces set them explicitly, so
/// ordinary test pages stay uncoloured by default.
const fixtureSpaceColors = ['apricot', 'paw', 'moss', 'sun', 'sky', 'duvet'];

/// Number of live top-level pages: the cycle index the next top-level page
/// used to get. Read it before creating the page.
Future<int> liveTopLevelCount(Vault vault) async =>
    (await vault.listChildren(null)).length;

/// Gives top-level page [id] the colour the old cycle gave it at [index].
Future<void> colourFixtureSpace(Vault vault, String id, int index) => vault.run(
  (v) => v.setPageColor(
    page: id,
    color: fixtureSpaceColors[index % fixtureSpaceColors.length],
  ),
  writes: true,
);

/// Creates a top-level fixture page with the colour the old cycle gave it.
Future<String> createFixtureSpace(Vault vault, {required String title}) async {
  final index = await liveTopLevelCount(vault);
  final id = await vault.createPage(title: title);
  await colourFixtureSpace(vault, id, index);
  return id;
}

/// One test's vault and app state.
class TestVault {
  TestVault(this.dir, this.app, this.pages);
  final Directory dir;
  final AppState app;

  /// Title -> page id for every fixture page.
  final Map<String, String> pages;

  Vault get vault => app.vault;
  String get showcaseId => pages[showcaseTitle]!;
  String? get bigId => pages['Big page'];
}

TestVault? _current;

/// The vault of the running test.
TestVault get current => _current!;

List<BOp> _bigPageOps(int n) {
  const kinds = [
    Kinds.paragraph,
    Kinds.paragraph,
    Kinds.heading2,
    Kinds.bulleted,
    Kinds.bulleted,
    Kinds.todo,
    Kinds.numbered, //
    Kinds.paragraph, Kinds.quote, Kinds.paragraph,
  ];
  const words = [
    'project',
    'meeting',
    'notes',
    'garden',
    'recipe',
    'travel',
    'budget',
    'design',
    'review',
    'plan',
  ];
  final ops = <BOp>[];
  String? after;
  for (var i = 0; i < n; i++) {
    final id = newId();
    ops.add(
      BOp(
        op: OpKind.insert,
        id: id,
        after: after,
        kind: kinds[i % kinds.length],
        text:
            'Block ${i + 1}: ${words[i % words.length]} ${words[(i * 7) % words.length]} text',
        marks: const [],
        checked: false,
      ),
    );
    after = id;
  }
  return ops;
}

/// The remembered interface state of a test vault: saved in its folder when
/// [persist], in memory otherwise.
UiState _uiFor(Directory dir, bool persist) => persist
    ? UiState.load(
        File('${dir.path}${Platform.pathSeparator}ui_state.json'),
        _vaultPath(dir),
      )
    : UiState();

String _vaultPath(Directory dir) => '${dir.path}${Platform.pathSeparator}vault';

Future<TestVault> _createVault({
  required bool big,
  bool persist = false,
  bool demo = false,
  bool empty = false,
  bool fresh = false,
  String? copyVault,
  DateTime? now,
}) async {
  resetTestClock();
  if (now != null) {
    Clock.fixedAt = now;
    rust_clock.setTestClock(startMs: now.millisecondsSinceEpoch);
  }
  final dir = Directory.systemTemp.createTempSync('notes_test_');
  if (copyVault != null) {
    final source = Directory(copyVault);
    if (!source.existsSync()) {
      throw TestFailure('Vault folder not found: $copyVault');
    }
    void copy(Directory from, Directory to) {
      to.createSync(recursive: true);
      for (final entity in from.listSync(followLinks: false)) {
        final name = entity.uri.pathSegments.where((s) => s.isNotEmpty).last;
        final destination = '${to.path}${Platform.pathSeparator}$name';
        if (entity is Directory) {
          copy(entity, Directory(destination));
        } else if (entity is File) {
          entity.copySync(destination);
        } else {
          throw TestFailure(
            'Vault copy contains a symbolic link: ${entity.path}',
          );
        }
      }
    }

    copy(source, Directory(_vaultPath(dir)));
  }
  final vault = await Vault.open(_vaultPath(dir));
  if (demo || empty || fresh || copyVault != null) {
    final pages = demo ? await createDemo(vault) : <String, String>{};
    // Same normal-new-vault seed as NotesApp._boot (SEED_TASKS == 0).
    if (fresh) {
      pages[gettingStartedTitle] = await createGettingStarted(
        vault,
        createTasks: true,
      );
    }
    await vault.run((v) => v.ensureSpaceColors(), writes: true);
    await vault.flush();
    final app = AppState(
      vault,
      reminderBackend: FakeReminderBackend(),
      ui: _uiFor(dir, persist),
      links: _links(),
    );
    await app.start();
    return TestVault(dir, app, pages);
  }
  final pages = <String, String>{};
  for (final t in fixtureTitles) {
    pages[t] = await createFixtureSpace(vault, title: t);
    await vault.apply(pages[t]!, [
      BOp(
        op: OpKind.insert,
        id: newId(),
        kind: Kinds.paragraph,
        text: 'Some text about ${t.toLowerCase()}.',
        marks: const [],
        checked: false,
      ),
    ]);
  }
  for (final t in recipeChildren) {
    pages[t] = await vault.createPage(parent: pages['Recipes'], title: t);
  }
  if (big) {
    final id = await createFixtureSpace(vault, title: 'Big page');
    await vault.apply(id, _bigPageOps(2000));
    pages['Big page'] = id;
  }
  final showcaseIndex = await liveTopLevelCount(vault);
  pages[showcaseTitle] = await createShowcase(vault);
  await colourFixtureSpace(vault, pages[showcaseTitle]!, showcaseIndex);
  await vault.flush();
  final app = AppState(
    vault,
    reminderBackend: FakeReminderBackend(),
    ui: _uiFor(dir, persist),
    links: _links(),
  );
  await app.start();
  return TestVault(dir, app, pages);
}

// ---------------------------------------------------------------------------
// Waiting for the core

/// Pumps frames and lets bridge calls finish until the app is quiet: no call
/// in flight, the change feed idle, no store reload scheduled (on the fake
/// or the real clock) and at least [min] of fake time passed.
Future<void> settle(WidgetTester tester, {Duration min = Duration.zero}) async {
  var quiet = 0;
  var elapsed = Duration.zero;
  const step = Duration(milliseconds: 50);
  final deadline = Stopwatch()..start();
  await tester.runAsync(() async {
    await _current?.app.feed.poll();
  });
  while (deadline.elapsed < const Duration(seconds: 30)) {
    await tester.runAsync(() => Future<void>.delayed(Duration.zero));
    await tester.pump(step);
    elapsed += step;
    final feedIdle = _current?.app.feed.idle ?? true;
    final dataBusy =
        Vault.inFlight.value > 0 || !feedIdle || Debounce.pending > 0;
    final busy = dataBusy || tester.binding.hasScheduledFrame;
    quiet = busy ? 0 : quiet + 1;
    if (quiet >= 3 && elapsed >= min) return;
    // Keep the old 30s fake animation bound (spinners never become quiet),
    // but never return with native work pending: zero-delay yields can pump
    // 600 frames before a disk flush finishes under parallel load.
    if (!dataBusy && elapsed >= const Duration(seconds: 30)) return;
  }
  throw TestFailure(
    'settle timed out: ${Vault.inFlight.value} bridge calls, '
    '${Debounce.pending} debounces pending${_bridgeTrace?.pendingReport() ?? ''}',
  );
}

/// The page's blocks as the core has them now.
Future<List<FlatBlock>> blocksOf(WidgetTester tester, String pageId) async {
  await settle(tester);
  final p = await tester.runAsync(() => current.vault.loadPage(pageId));
  return p!.blocks;
}

Future<FlatBlock> blockById(
  WidgetTester tester,
  String pageId,
  String id,
) async => (await blocksOf(tester, pageId)).firstWhere((b) => b.id == id);

Future<PageData> pageOf(WidgetTester tester, String pageId) async {
  await settle(tester);
  return (await tester.runAsync(() => current.vault.loadPage(pageId)))!;
}

/// Runs a core call from the test (for set-up or checks).
Future<T> core<T>(WidgetTester tester, Future<T> Function(Vault v) f) async {
  final r = await tester.runAsync(() => f(current.vault));
  return r as T;
}

// ---------------------------------------------------------------------------
// The app

class _FakeKeyboard extends StatelessWidget {
  const _FakeKeyboard({required this.dark});
  final bool dark;
  @override
  Widget build(BuildContext context) {
    final bg = dark ? const Color(0xFF1F1F1F) : const Color(0xFFE8EAED);
    final key = dark ? const Color(0xFF3A3A3A) : const Color(0xFFFFFFFF);
    final rows = [10, 9, 9, 5];
    return Container(
      color: bg,
      padding: const EdgeInsets.fromLTRB(4, 10, 4, 10),
      child: Column(
        children: [
          for (final n in rows)
            Expanded(
              child: Row(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  for (var i = 0; i < n; i++)
                    Expanded(
                      flex: n == 5 && i == 2 ? 5 : 1,
                      child: Container(
                        margin: const EdgeInsets.all(3),
                        decoration: BoxDecoration(
                          color: key,
                          borderRadius: BorderRadius.circular(6),
                        ),
                      ),
                    ),
                ],
              ),
            ),
        ],
      ),
    );
  }
}

const keyboardHeight = 280.0;

/// Sets form factor, fonts, theme and window size, creates the test vault
/// and pumps the app on the home screen. With [persist] the interface state
/// is saved in the vault's folder, so [restartApp] reads it back.
Future<TestVault> pumpApp(
  WidgetTester tester, {
  required bool mobile,
  bool dark = false,
  bool big = false,
  bool persist = false,
  bool demo = false,
  bool empty = false,
  bool fresh = false,
  String? copyVault,
  DateTime? now,
  RunnerAssignments? runners,
}) async {
  FileDialogs.instance = FakeFileDialogs();
  fakeLinks.clear();
  fetchedLinks.clear();
  await tester.runAsync(loadFonts);
  sansFamily = 'Atkinson Hyperlegible Next';
  monoFamily = 'Fragment Mono';
  debugDefaultTargetPlatformOverride = mobile
      ? TargetPlatform.android
      : TargetPlatform.windows;
  debugDisableShadows = false;
  Debounce.pending = 0; // none can be left from an earlier test's zone
  // A steady caret: its blink phase would depend on how long settle pumped.
  EditableText.debugDeterministicCursor = true;
  final dpr = mobile ? 2.0 : 1.0;
  tester.view.devicePixelRatio = dpr;
  tester.view.physicalSize = mobile
      ? const Size(375 * 2, 812 * 2)
      : const Size(1440, 900);
  tester.view.padding = mobile
      ? const FakeViewPadding(top: 24 * 2, bottom: 16 * 2)
      : FakeViewPadding.zero;
  tester.view.viewPadding = tester.view.padding;
  tester.view.viewInsets = FakeViewPadding.zero;
  tester.platformDispatcher.platformBrightnessTestValue = dark
      ? Brightness.dark
      : Brightness.light;
  addTearDown(() {
    tester.view.reset();
    tester.platformDispatcher.clearAllTestValues();
    Clock.fixedAt = null;
    EditableText.debugDeterministicCursor = false;
  });

  final tv = (await tester.runAsync(
    () => _createVault(
      big: big,
      persist: persist,
      demo: demo,
      empty: empty,
      fresh: fresh,
      copyVault: copyVault,
      now: now,
    ),
  ))!;
  _current = tv;
  await _pumpShell(tester, tv, mobile: mobile, dark: dark, runners: runners);
  return tv;
}

/// Closes the app and its vault, then opens the same vault folder (and the
/// same saved interface state) again, as a restart of the app would.
Future<TestVault> restartApp(
  WidgetTester tester, {
  bool restoreLastPage = true,
}) async {
  final old = current;
  final mobile = debugDefaultTargetPlatformOverride == TargetPlatform.android;
  final dark = tester.platformDispatcher.platformBrightness == Brightness.dark;
  await tester.pumpWidget(const SizedBox.shrink());
  await settle(tester);
  await tester.runAsync(() => old.app.vault.flush());
  // Disposing waits for the last page save, a future of the test zone: run
  // it there and pump until it is done.
  var closed = false;
  old.app.dispose().then((_) => closed = true);
  for (var i = 0; i < 20 && !closed; i++) {
    await settle(tester, min: Duration.zero);
  }
  final tv = (await tester.runAsync(() async {
    final vault = await Vault.open(_vaultPath(old.dir));
    final app = AppState(
      vault,
      reminderBackend: FakeReminderBackend(),
      ui: _uiFor(old.dir, old.app.ui.file != null),
      links: _links(),
    );
    await app.start();
    return TestVault(old.dir, app, old.pages);
  }))!;
  _current = tv;
  await _pumpShell(
    tester,
    tv,
    mobile: mobile,
    dark: dark,
    restoreLastPage: restoreLastPage,
  );
  return tv;
}

/// Gives the board a runner picker when a test passes its assignments.
Widget _withRunners(RunnerAssignments? runners, Widget child) =>
    runners == null ? child : RunnerScope(assignments: runners, child: child);

Future<void> _pumpShell(
  WidgetTester tester,
  TestVault tv, {
  required bool mobile,
  required bool dark,
  bool restoreLastPage = false,
  RunnerAssignments? runners,
}) async {
  await tester.pumpWidget(
    MediaQuery.fromView(
      view: tester.view,
      child: Directionality(
        textDirection: TextDirection.ltr,
        child: Stack(
          children: [
            Positioned.fill(
              child: NotesShell(
                navigatorKey: GlobalKey<NavigatorState>(),
                wrap: (child) => _withRunners(
                  runners,
                  SettingsScope(
                    settings: tv.app.settings,
                    child: AppScope(state: tv.app, child: child),
                  ),
                ),
                home: HomeScreen(restoreLastPage: restoreLastPage),
              ),
            ),
            if (mobile) _KeyboardHost(dark: dark),
          ],
        ),
      ),
    ),
  );
  await settle(tester);
}

/// Completes real work and fake-zone continuations without building frames.
/// Awaiting a previously started fake-zone future inside runAsync can deadlock.
Future<T> finishAsync<T>(WidgetTester tester, Future<T> future) async {
  var done = false;
  T? result;
  Object? error;
  StackTrace? stack;
  future.then<void>(
    (value) {
      result = value;
      done = true;
    },
    onError: (Object e, StackTrace trace) {
      error = e;
      stack = trace;
      done = true;
    },
  );
  final deadline = Stopwatch()..start();
  while (!done && deadline.elapsed < const Duration(seconds: 30)) {
    await tester.runAsync(() => Future<void>.delayed(Duration.zero));
    await tester.idle();
  }
  if (!done) {
    throw TestFailure('asynchronous test work did not finish within 30s');
  }
  if (error != null) Error.throwWithStackTrace(error!, stack!);
  return result as T;
}

/// Closes the app, then frees and deletes the test vault.
Future<void> _closeApp(WidgetTester tester) async {
  final tv = _current;
  if (tv == null) return;
  await tester.pumpWidget(const SizedBox.shrink());
  await settle(tester);
  await finishAsync(tester, tv.app.dispose());
  await tester.runAsync(() async {
    try {
      await tv.dir.delete(recursive: true);
    } catch (_) {
      // Windows may hold a file a moment longer; the temp folder is fine.
    }
  });
  _current = null;
  // Let any remaining fake timers fire (none should touch the core).
  await tester.pump(const Duration(seconds: 3));
}

/// Shows the stand-in keyboard whenever the view has a bottom inset.
class _KeyboardHost extends StatelessWidget {
  const _KeyboardHost({required this.dark});
  final bool dark;
  @override
  Widget build(BuildContext context) {
    final inset = MediaQuery.viewInsetsOf(context).bottom;
    if (inset <= 0) return const SizedBox.shrink();
    return Positioned(
      left: 0,
      right: 0,
      bottom: 0,
      height: inset,
      child: _FakeKeyboard(dark: dark),
    );
  }
}

/// Simulates the soft keyboard opening (mobile).
Future<void> showKeyboard(WidgetTester tester) async {
  tester.view.viewInsets = const FakeViewPadding(bottom: keyboardHeight * 2);
  await settle(tester);
}

// ---- finders and helpers ----

Finder editableWith(String text) => find.byWidgetPredicate(
  (w) => w is EditableText && w.controller.text == text && w.key is GlobalKey,
  description: 'block "$text"',
);

Finder editableStarting(String text) => find.byWidgetPredicate(
  (w) =>
      w is EditableText &&
      w.controller.text.startsWith(text) &&
      w.key is GlobalKey,
  description: 'block starting "$text"',
);

EditableTextState editState(WidgetTester tester, Finder f) =>
    tester.state<EditableTextState>(f);

/// The chip's label for [query] in the paragraph reading [text], or null
/// before its first result is in. Kept here, not in the add-on tests, which
/// may not import the editor.
String? chipLabelOf(WidgetTester tester, String text, String query) =>
    (editState(tester, editableWith(text)).widget.controller as RichController)
        .inlineLabelOf(BMark(start: 0, end: 0, kind: BMarkKind.query, url: query));

/// Focuses a block and puts the caret (or a selection) in it, as a tap would.
Future<void> focusAt(
  WidgetTester tester,
  Finder block,
  int offset, {
  int? extent,
}) async {
  final st = editState(tester, block);
  st.widget.focusNode.requestFocus();
  await tester.pump();
  st.widget.controller.selection = TextSelection(
    baseOffset: offset,
    extentOffset: extent ?? offset,
  );
  st.userUpdateTextEditingValue(
    st.widget.controller.value.copyWith(
      selection: TextSelection(
        baseOffset: offset,
        extentOffset: extent ?? offset,
      ),
    ),
    SelectionChangedCause.keyboard,
  );
  await settle(tester, min: Duration.zero);
}

/// Types [text] at the caret of the focused block, through the text input
/// connection like a keyboard would.
Future<void> typeText(WidgetTester tester, String text) async {
  for (final ch in text.split('')) {
    final st = tester.state<EditableTextState>(
      find.byWidgetPredicate(
        (w) => w is EditableText && w.focusNode.hasFocus && w.key is GlobalKey,
        description: 'focused block',
      ),
    );
    final v = st.widget.controller.value;
    final sel = v.selection.isValid
        ? v.selection
        : TextSelection.collapsed(offset: v.text.length);
    final nv = v.replaced(sel, ch);
    tester.testTextInput.updateEditingValue(nv);
    await tester.pump();
  }
  await settle(tester, min: Duration.zero);
}

/// Mobile: opens the page tree panel from the dock's tree button (Home stays
/// underneath).
Future<void> openMobileTree(WidgetTester tester) async {
  await tapAndSettle(tester, find.byKey(const ValueKey('nav-pages')));
}

/// Mobile: closes the tree panel with Android back (the system back event).
Future<void> closeMobileTreeWithBack(WidgetTester tester) async {
  await tester.binding.handlePopRoute();
  await settle(tester);
}

/// Mobile: closes the tree panel by tapping its scrim, the strip right of
/// the panel at mid-height.
Future<void> closeMobileTreeWithScrim(WidgetTester tester) async {
  final size = tester.view.physicalSize / tester.view.devicePixelRatio;
  await tester.tapAt(Offset(size.width - 8, size.height / 2));
  await settle(tester);
}

/// Mobile: opens Settings from the tree panel (Home stays underneath).
Future<void> openMobileSettings(WidgetTester tester) async {
  final settings = find.byKey(const ValueKey('pages-settings'));
  if (settings.evaluate().isEmpty) {
    await openMobileTree(tester);
  }
  await tester.ensureVisible(settings);
  await settle(tester);
  await tapAndSettle(tester, settings);
}

/// Taps [f] and waits for the app.
Future<void> tapAndSettle(
  WidgetTester tester,
  Finder f, {
  bool waitForBridge = false,
}) async {
  await tester.tap(f);
  if (waitForBridge) {
    // Snapshot selection changes before its native load finishes. Building
    // between those updates would bind the previous snapshot to the new key.
    final wait = Stopwatch()..start();
    do {
      await tester.runAsync(() => Future<void>.delayed(Duration.zero));
      // Bridge continuations belong to the fake zone. Flush that zone's
      // microtasks without building a frame with the intermediate snapshot.
      await tester.idle();
    } while (Vault.inFlight.value > 0 &&
        wait.elapsed < const Duration(seconds: 30));
    expect(
      Vault.inFlight.value,
      0,
      reason: 'bridge work completes before rebuilding',
    );
  }
  await settle(tester);
}

Future<void> openShowcase(WidgetTester tester) async {
  final mobile = tester.element(find.byType(HomeScreen)).mobile;
  if (mobile) await openMobileTree(tester);
  // Recent cards no longer appear in Today. Open the canonical page row.
  final page = find.byKey(
    ValueKey('${mobile ? 'pages-' : 'tree-'}${current.showcaseId}'),
  );
  await tester.ensureVisible(page);
  await settle(tester);
  await tester.tap(page);
  await settle(tester);
}

/// Opens home’s collapsed page browser and reveals its filter.
Future<void> expandAllPages(WidgetTester tester) async {
  final toggle = find.byKey(const ValueKey('all-pages-toggle'));
  await tester.ensureVisible(toggle);
  await settle(tester, min: Duration.zero);
  await tapAndSettle(tester, toggle);
  await tester.ensureVisible(find.byKey(const ValueKey('search-field')));
  await settle(tester, min: Duration.zero);
}

/// testWidgets with the app's set-up and tear-down: restores the debug
/// overrides pumpApp sets and closes the test vault.
void appTest(
  String description,
  Future<void> Function(WidgetTester tester) body, {
  bool skip = false,
  Timeout? timeout,
  List<String>? tags,
}) {
  testWidgets(
    description,
    (tester) async {
      try {
        await body(tester);
      } finally {
        await _closeApp(tester);
        debugDefaultTargetPlatformOverride = null;
        debugDisableShadows = true;
      }
    },
    skip: skip,
    timeout: timeout,
    tags: tags,
  );
}

/// Settles until [done] holds (for changes that arrive through the feed
/// from calls made by the test itself, whose follow-up timers run on the
/// real clock), up to [timeout] of real time.
Future<void> settleUntil(
  WidgetTester tester,
  bool Function() done, {
  Duration timeout = const Duration(seconds: 10),
}) async {
  // Drain UI work, then force the core's writer through its existing ack.
  await settle(tester, min: Duration.zero);
  await tester.runAsync(() async {
    await _current?.vault.flush();
  });
  final sw = Stopwatch()..start();
  while (!done() && sw.elapsed < timeout) {
    await tester.runAsync(() => Future<void>.delayed(Duration.zero));
    await settle(tester, min: Duration.zero);
  }
  // [done] may have turned true from a real timer after the last frame (the
  // sync status poll, for one): let the widgets catch up before returning.
  await settle(tester, min: Duration.zero);
}
