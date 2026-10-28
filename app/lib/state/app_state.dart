// Everything the UI reads, created once per open vault: the vault facade,
// the change feed and the stores. Provided to the widget tree by
// [AppScope]; widgets read stores with `context.read<T>()` and rebuild on
// their changes with `context.watch<T>()` (both backed by InheritedNotifier).

import 'package:flutter/widgets.dart';

import '../core/change_feed.dart';
import '../core/link_previews.dart';
import '../core/vault.dart';
import 'page_store.dart';
import 'pages_store.dart';
import 'sync_store.dart';
import 'tree_store.dart';
import 'ui_state.dart';
import 'editor_focus.dart';
import 'agenda_store.dart';
import 'reminder_store.dart';
import '../features/reminders/backends.dart';
import '../features/settings/settings_store.dart';
import '../addons/agent/api.dart';

class AppState {
  /// [ui] is this device's remembered interface state and [links] its
  /// bookmark previews (both in memory if null).
  AppState(
    this.vault, {
    UiState? ui,
    LinkPreviews? links,
    AppSettings? settings,
    ReminderBackend? reminderBackend,
  }) : feed = ChangeFeed(vault),
       settings = settings ?? AppSettings(),
       ui = ui ?? UiState(),
       links = links ?? LinkPreviews() {
    pages = PagesStore(vault, feed);
    tree = TreeStore(vault, feed, ui: this.ui);
    pageStore = PageStore(vault, feed);
    sync = SyncStore(vault, feed);
    agenda = AgendaStore(vault, feed);
    reminders = ReminderStore(
      vault,
      feed,
      this.settings,
      backend: reminderBackend,
    );
  }

  final Vault vault;
  final AppSettings settings;
  final ChangeFeed feed;
  final UiState ui;
  final LinkPreviews links;
  late final PagesStore pages;
  late final TreeStore tree;
  late final PageStore pageStore;
  late final SyncStore sync;
  late final AgendaStore agenda;
  late final ReminderStore reminders;

  /// Starts the change feed and loads what the first screen shows.
  Future<void> start() async {
    await feed.start();
    await pages.reload();
    await agenda.reload();
    await reminders.start();
    tree.childrenOf(null);
  }

  /// Stops polling, saves open pages and frees the stores (and the vault).
  Future<void> dispose() async {
    feed.stop();
    EditorFocus.disposeFor(this);
    await AgentStore.closeFor(vault);
    await reminders.close();
    await pageStore.lastSave;
    pages.dispose();
    tree.dispose();
    pageStore.dispose();
    sync.dispose();
    agenda.dispose();
    ui.dispose();
    links.dispose();
    vault.dispose();
  }
}

/// Provides [AppState] and each store to the tree.
class AppScope extends StatelessWidget {
  const AppScope({super.key, required this.state, required this.child});
  final AppState state;
  final Widget child;

  static AppState of(BuildContext context) =>
      context.getInheritedWidgetOfExactType<_AppStateScope>()!.state;

  /// The app state, or null before a vault is open.
  static AppState? maybeOf(BuildContext context) =>
      context.getInheritedWidgetOfExactType<_AppStateScope>()?.state;

  @override
  Widget build(BuildContext context) => _AppStateScope(
    state: state,
    child: _StoreScope<PagesStore>(
      notifier: state.pages,
      child: _StoreScope<TreeStore>(
        notifier: state.tree,
        child: _StoreScope<PageStore>(
          notifier: state.pageStore,
          child: _StoreScope<SyncStore>(
            notifier: state.sync,
            child: _StoreScope<AgendaStore>(
              notifier: state.agenda,
              child: _StoreScope<ReminderStore>(
                notifier: state.reminders,
                child: child,
              ),
            ),
          ),
        ),
      ),
    ),
  );
}

class _AppStateScope extends InheritedWidget {
  const _AppStateScope({required this.state, required super.child});
  final AppState state;
  @override
  bool updateShouldNotify(_AppStateScope old) => old.state != state;
}

class _StoreScope<T extends ChangeNotifier> extends InheritedNotifier<T> {
  const _StoreScope({super.notifier, required super.child});
}

extension StoreX on BuildContext {
  /// The store of type [T], rebuilding this widget when it notifies.
  T watch<T extends ChangeNotifier>() =>
      dependOnInheritedWidgetOfExactType<_StoreScope<T>>()!.notifier!;

  /// The store of type [T] if one is provided above, rebuilding this widget
  /// when it notifies; null before the vault is open.
  T? maybeWatch<T extends ChangeNotifier>() =>
      dependOnInheritedWidgetOfExactType<_StoreScope<T>>()?.notifier;

  /// The store of type [T], without rebuilding on its changes.
  T read<T extends ChangeNotifier>() =>
      getInheritedWidgetOfExactType<_StoreScope<T>>()!.notifier!;

  AppState get app => AppScope.of(this);
}
