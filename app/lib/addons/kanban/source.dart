// The kanban's BoardSource: reads the board from the page's type data, writes
// every change back through the vault, and refreshes from the change feed.
// Each write reads the stored board first, so positions come from what the
// vault holds (including changes made on another device). After each local
// write the readable Markdown body is rewritten from the same fresh state.

import 'dart:async';
import 'dart:convert';

import 'package:flutter/widgets.dart';

import '../../core/change_feed.dart';
import '../../core/vault.dart';
import '../../services/legacy_host.dart';
import '../../src/rust/api/types.dart';
import '../../state/app_state.dart';
import '../../state/pages_store.dart' show firstLineOfTexts;
import '../../ui/board/board_source.dart';
import '../../ui/database/db_theme.dart';
import '../../ui/ui.dart';
import 'add_card.dart';
import 'data.dart';
import 'markdown.dart';

class KanbanBoardSource extends BoardSource {
  KanbanBoardSource(this.app, this.pageId) {
    app.feed.addListener(_onChanges);
    app.pages.addListener(_onPages);
    _reload();
  }

  final AppState app;

  /// The kanban note's id.
  final String pageId;

  /// The column colour tokens' colours. The view sets these on each build.
  AppColors colors = AppColors.light;

  KanbanData _data = KanbanData.empty;
  bool _disposed = false;

  /// Fallback preview pages, read on demand. Registered card builders reuse
  /// metadata and do not need an extra body read.
  final _cardPages = <String, PageData>{};
  final _previewWanted = <String>{};

  /// The latest read started for each card; an older read that finishes
  /// later is dropped.
  final _pageTicket = <String, int>{};
  int _ticket = 0;

  /// The latest board read; an older one that finishes later is dropped.
  int _reloadGen = 0;

  /// The local writes, run one after another so each reads the board as the
  /// previous one left it and the readable body is never written from a
  /// stale board.
  Future<void> _queue = Future<void>.value();

  Vault get _vault => app.vault;

  @override
  void dispose() {
    _disposed = true;
    app.feed.removeListener(_onChanges);
    app.pages.removeListener(_onPages);
    super.dispose();
  }

  // Incoming changes only reload the view. They never rewrite the readable
  // body, so a synced body cannot start a write loop.
  void _onChanges(ChangeBatch batch) {
    if (_disposed || _vault.isClosed) return;
    if (batch.reset) _cardPages.clear();
    // A batch can change both the board and a card. Invalidate previews before
    // the board reload branch so it also refreshes those changed cards.
    for (final id in _cardPages.keys.toList()) {
      if (batch.touches(id)) {
        _cardPages.remove(id);
        _pageTicket[id] = ++_ticket;
      }
    }
    if (batch.reset || batch.treeChanged || batch.touches(pageId)) {
      _reload();
      return;
    }
    // A card page that changed: read it again for its first line.
    final touched = [
      for (final id in _previewWanted)
        if (batch.touches(id)) id,
    ];
    if (touched.isNotEmpty) _loadPages(touched);
  }

  // Page titles, trash and missing pages reach the board through the pages store.
  void _onPages() {
    if (!_disposed) notifyListeners();
  }

  Iterable<String> get _listedCards => _data.cards.values.expand((ids) => ids);

  Future<KanbanData> _read() async => KanbanData.parse(
    await _vault.typeData(pageId, kanbanAddonId),
    pageId: pageId,
  );

  Future<void> _reload() async {
    if (_disposed || _vault.isClosed) return;
    final gen = ++_reloadGen;
    final KanbanData data;
    try {
      data = await _read();
    } catch (_) {
      // The page may be gone or not yet readable; keep the last board shown.
      return;
    }
    // A newer read started after this one; its board is the one to show.
    if (_disposed || gen != _reloadGen) return;
    _data = data;
    final listed = {for (final id in _listedCards) id};
    _cardPages.removeWhere((id, _) => !listed.contains(id));
    _previewWanted.retainAll(listed);
    notifyListeners();
    final unread = [
      for (final id in _previewWanted)
        if (!_cardPages.containsKey(id)) id,
    ];
    if (unread.isNotEmpty) unawaited(_loadPages(unread));
  }

  /// Reads the pages of [ids]. A later read of the same card supersedes an
  /// earlier one that is still in flight.
  Future<void> _loadPages(List<String> ids) async {
    final tickets = <String, int>{};
    for (final id in ids) {
      tickets[id] = _pageTicket[id] = ++_ticket;
    }
    var changed = false;
    for (final id in ids) {
      if (_disposed || _vault.isClosed) return;
      final PageData page;
      try {
        page = await _vault.loadPage(id);
      } catch (_) {
        // Missing or unreadable: the card keeps showing its title alone.
        continue;
      }
      if (_disposed) return;
      if (_pageTicket[id] != tickets[id]) continue;
      if (!_listedCards.contains(id)) continue;
      _cardPages[id] = page;
      changed = true;
    }
    if (changed && !_disposed) notifyListeners();
  }

  // ---- what the board shows ----

  /// Card pages hidden while missing or in the trash; their ids stay stored.
  /// A page whose metadata is still loading is shown.
  bool _isShown(String id) {
    final meta = app.pages.meta(id);
    return meta == null || (!meta.missing && !meta.inTrash);
  }

  List<String> _shown(String columnId) => [
    for (final id in _data.cardsOf(columnId))
      if (_isShown(id)) id,
  ];

  BoardCard _card(String id) =>
      BoardCard(id: id, title: app.pages.meta(id)?.title ?? '');

  /// A card's first body line, from its page. Empty until the page has loaded.
  String firstLineOf(String cardId) {
    if (!_disposed && !_vault.isClosed && _previewWanted.add(cardId)) {
      unawaited(_loadPages([cardId]));
    }
    final page = _cardPages[cardId];
    return page == null ? '' : firstLineOfTexts(page.blocks.map((b) => b.text));
  }

  @override
  List<BoardColumn> get columns => [
    for (final column in _data.columns)
      BoardColumn(
        id: column.id,
        title: column.title.isEmpty ? 'Untitled' : column.title,
        count: cardCount(column.id),
        color: column.color == null ? null : colors.tone(column.color),
        background: column.color == null
            ? colors.bgSecondary
            : colors.groupTint(column.color),
      ),
  ];

  @override
  int cardCount(String columnId) => _shown(columnId).length;

  @override
  BoardCard? cardAt(String columnId, int index) {
    final ids = _shown(columnId);
    if (index < 0 || index >= ids.length) return null;
    return _card(ids[index]);
  }

  @override
  Iterable<BoardCard> loadedCards(String columnId) =>
      _shown(columnId).map(_card);

  // ---- writes ----

  List<String> _cardsPath(String columnId) => ['cards', columnId];

  /// Runs one local write sequence after the sequences before it. Every
  /// local board write goes through here, and ends by refreshing the body.
  Future<void> _serial(Future<void> Function() write) {
    final run = _queue.then((_) async {
      if (_disposed || _vault.isClosed) return;
      await write();
    });
    _queue = run.catchError((Object _) {});
    return run;
  }

  /// After a local board write: the readable body follows the board, then the
  /// view reloads. The reload runs even if the body write fails.
  Future<void> _settle() async {
    try {
      await _writeProjection();
    } finally {
      await _reload();
    }
  }

  /// Rewrites the readable body from the stored board. Each card links by its
  /// page id, labelled with the page's title as the vault holds it now;
  /// missing and trashed cards are left out, as the board hides them. Card
  /// renames made elsewhere reach the body at the next local board write.
  Future<void> _writeProjection() async {
    final data = await _read();
    final columns = <KanbanBodyColumn>[];
    for (final column in data.columns) {
      final links = <String>[];
      for (final id in data.cardsOf(column.id)) {
        final page = await _pageOrNull(id);
        if (page == null || page.inTrash) continue;
        links.add(kanbanCardLink(id, page.title));
      }
      columns.add(KanbanBodyColumn(column.title, links));
    }
    await _vault.setTypeMarkdownBody(pageId, kanbanReadableBody(columns));
  }

  Future<PageData?> _pageOrNull(String id) async {
    try {
      return await _vault.loadPage(id);
    } catch (_) {
      return null;
    }
  }

  /// Gives a column an empty card list.
  Future<void> _startCardList(String columnId) =>
      _vault.typeDataSet(pageId, kanbanAddonId, _cardsPath(columnId), '[]');

  /// Inserts [cardId] into [columnId] at [index] (the end when null).
  Future<void> _insertCard(
    KanbanData data,
    String columnId,
    String cardId,
    int? index,
  ) async {
    if (data.cards[columnId] == null) await _startCardList(columnId);
    await _vault.typeDataListInsert(
      pageId,
      kanbanAddonId,
      _cardsPath(columnId),
      jsonEncode(cardId),
      index: index,
    );
  }

  /// Moves a card from a board read taken just before. Writes nothing when the
  /// card or target column is missing, or the card is already in place.
  Future<void> _moveCard(
    KanbanData data,
    String cardId,
    String fromColumnId,
    String toColumnId,
    String? afterCardId,
  ) async {
    final from = data.cardsOf(fromColumnId);
    if (!from.contains(cardId)) return;
    if (!data.columns.any((column) => column.id == toColumnId)) return;
    if (fromColumnId == toColumnId) {
      final rest = [...from]..remove(cardId);
      final to = kanbanPositionAfter(rest, afterCardId);
      if (to == from.indexOf(cardId)) return;
      await _vault.typeDataListMove(
        pageId,
        kanbanAddonId,
        _cardsPath(fromColumnId),
        TypeDataListItem(id: cardId),
        to,
      );
    } else {
      await _vault.typeDataListDelete(
        pageId,
        kanbanAddonId,
        _cardsPath(fromColumnId),
        TypeDataListItem(id: cardId),
      );
      await _insertCard(
        data,
        toColumnId,
        cardId,
        kanbanPositionAfter(data.cardsOf(toColumnId), afterCardId),
      );
    }
  }

  @override
  Future<void> moveCard(
    String cardId,
    String fromColumnId,
    String toColumnId,
    String? afterCardId,
  ) => _serial(() async {
    if (afterCardId == cardId) return;
    await _moveCard(
      await _read(),
      cardId,
      fromColumnId,
      toColumnId,
      afterCardId,
    );
    await _settle();
  });

  /// Moves a card to the end of another column: the phone's Move to column
  /// entry. The end is read from the board inside the write, not from the
  /// caller's snapshot.
  Future<void> moveCardToEnd(
    String cardId,
    String fromColumnId,
    String toColumnId,
  ) => _serial(() async {
    if (fromColumnId == toColumnId) return;
    final data = await _read();
    final ids = data.cardsOf(toColumnId);
    await _moveCard(
      data,
      cardId,
      fromColumnId,
      toColumnId,
      ids.isEmpty ? null : ids.last,
    );
    await _settle();
  });

  @override
  Future<void> addCard(BuildContext context, String columnId) async {
    final choice = await showKanbanAddPanel(context);
    if (choice == null || !context.mounted) return;
    if (choice.existing) {
      final picked = await pickPage(
        context,
        globalRectOf(context),
        exclude: pageId,
      );
      if (picked == null) return;
      await _serial(() => _addExisting(columnId, picked));
    } else {
      final title = choice.title.trim();
      if (title.isEmpty) return;
      final id = await _vault.createPage(parent: pageId, title: title);
      await _serial(() => _append(columnId, id));
    }
  }

  /// Appends a note that already exists, unless it is this board or is
  /// already on it.
  Future<void> _addExisting(String columnId, String noteId) async {
    final data = await _read();
    if (noteId == pageId) return;
    if (data.cards.values.any((ids) => ids.contains(noteId))) return;
    await _insertCard(data, columnId, noteId, null);
    await _settle();
  }

  Future<void> _append(String columnId, String cardId) async {
    final data = await _read();
    await _insertCard(data, columnId, cardId, null);
    await _settle();
  }

  /// Removes the reference to [cardId] only; the note itself stays.
  Future<void> removeCard(String columnId, String cardId) => _serial(() async {
    final data = await _read();
    if (!data.cardsOf(columnId).contains(cardId)) return;
    await _vault.typeDataListDelete(
      pageId,
      kanbanAddonId,
      _cardsPath(columnId),
      TypeDataListItem(id: cardId),
    );
    await _settle();
  });

  @override
  void openCard(BuildContext context, BoardCard card) =>
      openPage(context, card.id, card.title);

  // ---- columns ----

  @override
  Future<void> Function(String title)? get addColumn => _addColumn;

  /// A new column starts with an empty card list.
  Future<void> _addColumn(String title) => _serial(() async {
    final id = newKanbanColumnId();
    await _vault.typeDataListInsert(pageId, kanbanAddonId, const [
      'columns',
    ], jsonEncode({'id': id, 'title': title}));
    await _startCardList(id);
    await _settle();
  });

  @override
  Future<void> Function(String columnId, String title)? get renameColumn =>
      _renameColumn;

  Future<void> _renameColumn(String columnId, String title) =>
      _serial(() async {
        final next = title.trim();
        if (next.isEmpty) return;
        await _vault.typeDataSet(pageId, kanbanAddonId, [
          'columns',
          columnId,
          'title',
        ], jsonEncode(next));
        await _settle();
      });

  @override
  Future<void> Function(String columnId, String? afterColumnId)?
  get reorderColumn => _reorderColumn;

  Future<void> _reorderColumn(String columnId, String? afterColumnId) =>
      _serial(() async {
        if (afterColumnId == columnId) return;
        final data = await _read();
        final ids = [for (final column in data.columns) column.id];
        final from = ids.indexOf(columnId);
        if (from < 0) return;
        final rest = [...ids]..remove(columnId);
        final to = kanbanPositionAfter(rest, afterColumnId);
        if (to == from) return;
        await _vault.typeDataListMove(
          pageId,
          kanbanAddonId,
          const ['columns'],
          TypeDataListItem(id: columnId),
          to,
        );
        await _settle();
      });

  /// Removes a column and moves its cards to the end of the first remaining
  /// column. The last column cannot be removed. No note is deleted.
  @override
  Future<void> Function(String columnId)? get deleteColumn => _deleteColumn;

  Future<void> _deleteColumn(String columnId) => _serial(() async {
    final data = await _read();
    final remaining = [
      for (final column in data.columns)
        if (column.id != columnId) column.id,
    ];
    if (remaining.isEmpty || remaining.length == data.columns.length) return;
    final moved = data.cardsOf(columnId);
    if (moved.isNotEmpty) {
      final target = remaining.first;
      if (data.cards[target] == null) await _startCardList(target);
      // Each card is appended with a list insert, so cards added to the target
      // column since this board was read are kept.
      for (final cardId in moved) {
        await _vault.typeDataListInsert(
          pageId,
          kanbanAddonId,
          _cardsPath(target),
          jsonEncode(cardId),
        );
      }
    }
    await _vault.typeDataListDelete(pageId, kanbanAddonId, const [
      'columns',
    ], TypeDataListItem(id: columnId));
    if (data.cards.containsKey(columnId)) {
      await _vault.typeDataDelete(pageId, kanbanAddonId, _cardsPath(columnId));
    }
    await _settle();
  });
}
