// The Dart side of the bridge: one open vault. Every call goes through
// flutter_rust_bridge and runs on a Rust worker thread, never on the UI
// isolate.
//
// - Common calls have named methods; everything else in the core API is
//   reachable through [run] (for example `vault.run((v) => v.listViews(
//   collection: id))`). Pass `writes: true` for calls that change the vault.
// - After a write the change feed is woken, so stores refresh right away
//   instead of on the next poll.
// - [inFlight] counts calls that have not completed (tests wait for it to
//   reach zero).

import 'package:flutter/foundation.dart';

import '../src/rust/api/agent.dart' show AgentSchedulePreview, AgentScheduleRun, AgentScheduleState;
import '../src/rust/api/types.dart';
import '../src/rust/api/vault.dart';

export '../src/rust/api/agent.dart'
    show AgentScheduleOccurrence, AgentSchedulePreview, AgentScheduleRun, AgentScheduleState;

class Vault {
  Vault(this.raw);

  /// The bridge handle, for calls without a named method (prefer [run]).
  final NotesVault raw;

  /// Calls started through any [Vault] and not completed yet.
  static final inFlight = ValueNotifier<int>(0);

  /// Called after every completed write (the change feed wakes on it).
  VoidCallback? onWrite;

  bool _closed = false;
  final _closeListeners = <VoidCallback>[];

  /// True before close listeners run and the native handle is released.
  bool get isClosed => _closed;

  /// Cancels vault-owned background work synchronously at close.
  void addCloseListener(VoidCallback listener) {
    if (_closed) {
      listener();
    } else {
      _closeListeners.add(listener);
    }
  }

  void removeCloseListener(VoidCallback listener) => _closeListeners.remove(listener);

  static Future<Vault> open(String root, {String? dataDir}) async =>
      Vault(await _count(() => NotesVault.open(root: root, dataDir: dataDir)));

  static Future<T> _count<T>(Future<T> Function() f) async {
    inFlight.value++;
    try {
      return await f();
    } finally {
      inFlight.value--;
    }
  }

  /// Runs one bridge call, counted in [inFlight]; wakes the feed after a write.
  Future<T> run<T>(Future<T> Function(NotesVault v) f, {bool writes = false}) async {
    final r = await _count(() => f(raw));
    if (writes) onWrite?.call();
    return r;
  }

  String get root => raw.root();
  String get deviceId => raw.deviceId();

  // ---- pages ----

  Future<int> pageCount() => run((v) => v.pageCount());

  /// Pages by last update, newest first; [parent] null = all pages.
  Future<List<PageRow>> listPages({String? parent, required int offset, required int limit}) =>
      run((v) => v.listPages(parent: parent, offset: offset, limit: limit));

  /// Children of [parent] (null = top level) in sibling order.
  Future<List<PageRow>> listChildren(String? parent, {int offset = 0, int limit = 500}) =>
      run((v) => v.listChildren(parent: parent, offset: offset, limit: limit));

  Future<List<Hit>> search(String query, {int limit = 100}) => run((v) => v.search(query: query, limit: limit));

  Future<String> createPage({
    String? parent,
    String title = '',
    String? pageType,
  }) => run(
    (v) => v.createPage(parent: parent, title: title, pageType: pageType),
    writes: true,
  );

  Future<void> setPageType(String page, String pageType) =>
      run((v) => v.setPageType(page: page, pageType: pageType), writes: true);

  /// JSON object for one add-on; an absent area is `{}`.
  Future<String> typeData(String page, String addon) =>
      run((v) => v.typeData(page: page, addon: addon));

  /// Checks a definition locally, including on devices without an agent.
  Future<AgentSchedulePreview> schedulePreview(String json) =>
      run((v) => v.schedulePreview(json: json));

  Future<void> localRunnerSetAvailable(bool available) =>
      run((v) => v.localRunnerSetAvailable(available: available), writes: true);

  Future<AgentScheduleState> localScheduleState(String pageId) =>
      run((v) => v.localScheduleState(pageId: pageId));

  Future<AgentScheduleState> localScheduleAccept(String pageId, String hash) =>
      run((v) => v.localScheduleAccept(pageId: pageId, hash: hash), writes: true);

  Future<AgentScheduleState> localSchedulePause(String pageId, bool paused) =>
      run((v) => v.localSchedulePause(pageId: pageId, paused: paused), writes: true);

  Future<AgentScheduleRun> localScheduleRunNow(String pageId) =>
      run((v) => v.localScheduleRunNow(pageId: pageId), writes: true);

  Future<String?> localNodeId() => run((v) => v.localNodeId());

  Future<String> typeDataAll(String page) =>
      run((v) => v.typeDataAll(page: page));

  /// All dated sources, including mentions. Timed bounds are instants;
  /// whole-day bounds are UTC calendar midnights.
  Future<List<AgendaItem>> calendarItems({
    required int timedFromMs,
    required int timedToMs,
    required int dayFromMs,
    required int dayToMs,
  }) => run(
    (v) => v.calendarItems(
      timedFromMs: timedFromMs,
      timedToMs: timedToMs,
      dayFromMs: dayFromMs,
      dayToMs: dayToMs,
    ),
  );

  /// Changes only a dated to-do mark's day. Pass [markRef] from its calendar
  /// item unchanged; [newDayMs] is a UTC calendar midnight.
  Future<void> moveDateMark({
    required String page,
    required String block,
    required DateMarkRef markRef,
    required int newDayMs,
  }) => run(
    (v) => v.moveDateMark(
      page: page,
      block: block,
      markRef: markRef,
      newDayMs: newDayMs,
    ),
    writes: true,
  );

  // ---- calendars ----

  /// Links a task under the day's Tasks toggle; false if already linked.
  Future<bool> linkTaskToDay({required int dayMs, required String taskPageId}) =>
      run((v) => v.linkTaskToDay(dayMs: dayMs, taskPageId: taskPageId), writes: true);

  /// Appends a timestamped line to the day's Log and returns the note id.
  Future<String> appendLog({required int dayMs, required String text}) =>
      run((v) => v.appendLog(dayMs: dayMs, text: text), writes: true);

  /// Sorted days with a Journal note in [fromMs, toMs), as UTC midnights.
  Future<List<int>> dayNotesInRange({required int fromMs, required int toMs}) =>
      run((v) async => (await v.dayNotesInRange(fromMs: fromMs, toMs: toMs))
          .map((day) => day.toInt()).toList());

  /// Calendar categories and database visibility. The first call creates the
  /// default calendars, so it counts as a write.
  Future<CalendarSettings> listCalendars() => run((v) => v.listCalendars(), writes: true);

  /// Adds a calendar at the end of the list and returns its id.
  Future<String> createCalendar({
    required String name,
    required String color,
    required bool showInCalendar,
    required bool showInToday,
  }) => run(
    (v) => v.createCalendar(
      name: name,
      color: color,
      showInCalendar: showInCalendar,
      showInToday: showInToday,
    ),
    writes: true,
  );

  Future<void> updateCalendar(CalendarCategory calendar) =>
      run((v) => v.updateCalendar(calendar: calendar), writes: true);

  Future<void> deleteCalendar(String id) => run((v) => v.deleteCalendar(id: id), writes: true);

  /// [ids] must list every calendar id exactly once.
  Future<void> reorderCalendars(List<String> ids) => run((v) => v.reorderCalendars(ids: ids), writes: true);

  Future<void> setDatabaseCalendarVisibility(String database, {required bool showInCalendar}) => run(
    (v) => v.setDatabaseCalendarVisibility(database: database, showInCalendar: showInCalendar),
    writes: true,
  );

  /// The ready-made readable Markdown body in the type-data root.
  Future<String?> typeMarkdownBody(String page) =>
      run((v) => v.typeMarkdownBody(page: page));

  /// Stores a readable body; null removes it.
  Future<void> setTypeMarkdownBody(String page, String? body) =>
      run((v) => v.setTypeMarkdownBody(page: page, body: body), writes: true);

  /// The same Markdown, including front matter, as the core vault writer.
  Future<String> exportMarkdown(String page) =>
      run((v) => v.exportMarkdown(page: page));

  Future<void> typeDataSet(
    String page,
    String addon,
    List<String> path,
    String jsonValue,
  ) => run(
    (v) => v.typeDataSet(
      page: page,
      addon: addon,
      path: path,
      jsonValue: jsonValue,
    ),
    writes: true,
  );

  Future<void> typeDataDelete(String page, String addon, List<String> path) =>
      run(
        (v) => v.typeDataDelete(page: page, addon: addon, path: path),
        writes: true,
      );

  Future<void> typeDataListInsert(
    String page,
    String addon,
    List<String> path,
    String jsonValue, {
    int? index,
  }) => run(
    (v) => v.typeDataListInsert(
      page: page,
      addon: addon,
      path: path,
      index: index,
      jsonValue: jsonValue,
    ),
    writes: true,
  );

  Future<void> typeDataListDelete(
    String page,
    String addon,
    List<String> path,
    TypeDataListItem item,
  ) => run(
    (v) =>
        v.typeDataListDelete(page: page, addon: addon, path: path, item: item),
    writes: true,
  );

  Future<void> typeDataListMove(
    String page,
    String addon,
    List<String> path,
    TypeDataListItem item,
    int toIndex,
  ) => run(
    (v) => v.typeDataListMove(
      page: page,
      addon: addon,
      path: path,
      item: item,
      toIndex: toIndex,
    ),
    writes: true,
  );

  Future<void> renamePage(String id, String title) => run((v) => v.renamePage(id: id, title: title), writes: true);

  /// Moves the page and its subtree to the trash.
  Future<void> deletePage(String id) => run((v) => v.deletePage(id: id), writes: true);

  Future<void> restorePage(String id) => run((v) => v.restorePage(id: id), writes: true);

  Future<void> movePage(String id, {String? newParent, String? after}) =>
      run((v) => v.movePage(id: id, newParent: newParent, after: after), writes: true);

  Future<String> duplicatePage(String id) => run((v) => v.duplicatePage(id: id), writes: true);

  Future<PageData> loadPage(String id) => run((v) => v.loadPage(id: id));

  /// One transaction: all ops or none.
  Future<void> apply(String pageId, List<BOp> ops) => run((v) => v.apply(pageId: pageId, ops: ops), writes: true);

  Future<void> setIcon(String id, String? icon) => run((v) => v.setIcon(id: id, icon: icon), writes: true);

  Future<void> setFavorite(String id, bool favorite) =>
      run((v) => v.setFavorite(id: id, favorite: favorite), writes: true);

  Future<List<PageRow>> listFavorites() => run((v) => v.listFavorites());

  Future<List<TrashedPage>> listTrash() => run((v) => v.listTrash());

  Future<List<PageRow>> backlinks(String id) => run((v) => v.backlinks(id: id));

  /// Waits until everything written so far is on disk.
  Future<void> flush() => run((v) => v.flush());

  // ---- change feed ----

  Future<ChangeBatch> pollChanges(int sinceSeq) => run((v) => v.pollChanges(sinceSeq: sinceSeq));

  /// Frees the native handle (the vault closes when nothing else holds it).
  void dispose() {
    if (_closed) return;
    _closed = true;
    onWrite = null;
    for (final listener in List.of(_closeListeners)) {
      listener();
    }
    _closeListeners.clear();
    raw.dispose();
  }
}
