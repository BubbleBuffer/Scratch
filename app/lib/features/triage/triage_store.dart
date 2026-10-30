// Triage proposals for the UI: the bridge calls behind an interface (tests
// pass a fake), and one store per vault that holds the list and the pending
// count. The store reloads when the change feed reports a change to the page
// that holds the proposals (`triage_page_id`).
//
// The proposals page may not exist yet (no run has proposed anything).
// Reading the list never creates it, but `triage_page_id` can, so the store
// asks for the id only once a read has found proposals. Until then it
// re-reads on every batch, which is a read-only call.

import 'package:flutter/foundation.dart';

import '../../core/change_feed.dart';
import '../../core/vault.dart';
import '../../src/rust/api/triage.dart';
import '../../src/rust/api/types.dart' show ChangeBatch;
import '../../state/app_state.dart';

export '../../src/rust/api/triage.dart' show TriageProposalDto;

/// Proposal statuses (`TriageProposalDto.status`).
abstract final class TriageStatus {
  static const pending = 'pending';
  static const accepted = 'accepted';
  static const dismissed = 'dismissed';
}

abstract interface class TriageBridge {
  /// Starts a triage run on [runner] and returns its AI task id.
  Future<String> start({
    String? projectId,
    List<String> pageIds = const [],
    required String runner,
  });

  /// Every proposal, newest first.
  Future<List<TriageProposalDto>> all();
  Future<TriageProposalDto> accept(String id);
  Future<TriageProposalDto> dismiss(String id);
  Future<TriageProposalDto> undo(String id);

  /// The page whose change means the proposals changed.
  Future<String> pageId();
}

class VaultTriageBridge implements TriageBridge {
  const VaultTriageBridge(this.vault);
  final Vault vault;

  @override
  Future<String> start({
    String? projectId,
    List<String> pageIds = const [],
    required String runner,
  }) => vault.run(
    (v) => triageStart(
      vault: v,
      scopeProjectId: projectId,
      pageIds: pageIds,
      runner: runner,
    ),
    writes: true,
  );

  @override
  Future<List<TriageProposalDto>> all() =>
      vault.run((v) => triageAllProposals(vault: v));

  @override
  Future<TriageProposalDto> accept(String id) =>
      vault.run((v) => triageAccept(vault: v, id: id), writes: true);

  @override
  Future<TriageProposalDto> dismiss(String id) =>
      vault.run((v) => triageDismiss(vault: v, id: id), writes: true);

  @override
  Future<TriageProposalDto> undo(String id) =>
      vault.run((v) => triageUndo(vault: v, id: id), writes: true);

  @override
  Future<String> pageId() => vault.run((v) => triagePageId(vault: v));
}

class TriageStore extends ChangeNotifier {
  TriageStore(this.bridge, {this.feed}) {
    feed?.addListener(onBatch);
    load();
  }

  final TriageBridge bridge;
  final ChangeFeed? feed;

  static final _stores = Expando<TriageStore>();

  /// The store of [app]'s vault.
  static TriageStore of(AppState app) => _stores[app.vault] ??= TriageStore(
    VaultTriageBridge(app.vault),
    feed: app.feed,
  );

  List<TriageProposalDto> _items = const [];
  bool _loaded = false, _disposed = false;
  String? _pageId;
  int _gen = 0;

  /// Proposals still waiting for a decision, and merged ones that can be
  /// undone, newest first. Dismissed proposals are left out.
  List<TriageProposalDto> get items => _items;
  bool get loaded => _loaded;

  /// The badge count: proposals waiting for a decision.
  int get pending =>
      _items.where((p) => p.status == TriageStatus.pending).length;

  @visibleForTesting
  void onBatch(ChangeBatch b) {
    final id = _pageId;
    if (id == null || b.touches(id)) load();
  }

  Future<void> load() async {
    final gen = ++_gen;
    try {
      final all = await bridge.all();
      if (_disposed || gen != _gen) return;
      _items = [
        for (final p in all)
          if (p.status != TriageStatus.dismissed) p,
      ];
      _loaded = true;
      if (_pageId == null && all.isNotEmpty) {
        _pageId = await bridge.pageId();
      }
    } catch (e) {
      debugPrint('triage proposals failed: $e');
      if (_disposed || gen != _gen) return;
      _loaded = true;
    }
    if (!_disposed) notifyListeners();
  }

  Future<void> merge(String id) => _decide(bridge.accept(id));
  Future<void> dismiss(String id) => _decide(bridge.dismiss(id));
  Future<void> undoMerge(String id) => _decide(bridge.undo(id));

  Future<void> _decide(Future<TriageProposalDto> call) async {
    try {
      await call;
    } finally {
      await load();
    }
  }

  @override
  void dispose() {
    _disposed = true;
    feed?.removeListener(onBatch);
    super.dispose();
  }
}
