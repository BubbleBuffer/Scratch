import 'package:flutter/widgets.dart';

import '../../core/models.dart';
import '../../ui/database/config.dart';
import '../../ui/database/db_theme.dart';
import '../../ui/database/values.dart';
import '../../registry/view_registry.dart';
import '../../state/collection_store.dart';
import '../../ui/board/board_source.dart';
import '../../ui/ui.dart';

/// Adapts a live database query to the storage-independent board interface.
/// Property/schema editing remains in the database's existing menus.
class DatabaseBoardSource extends BoardSource {
  DatabaseBoardSource(this.args) {
    args.store.addListener(_changed);
  }

  ViewArgs args;
  AppColors colors = AppColors.light;
  bool _disposed = false;

  void _changed() => notifyListeners();

  @override
  void dispose() {
    _disposed = true;
    args.store.removeListener(_changed);
    super.dispose();
  }

  GroupSource group(String id) =>
      args.store.groups.firstWhere((g) => g.key == id);

  String? label(GroupSource group) {
    final def = args.store.groupDef!;
    if (group.key.isEmpty) return null;
    if (def.kind == PropKind.checkbox) {
      return group.key == 'true' ? 'Checked' : 'Unchecked';
    }
    return group.group.label;
  }

  @override
  List<BoardColumn> get columns {
    final store = args.store;
    final def = store.groupDef;
    if (def == null) return const [];
    final cfg = args.view.cfg;
    final ordered = def.kind == PropKind.status
        ? [
            ...store.groups.where((g) => g.key.isNotEmpty),
            ...store.groups.where((g) => g.key.isEmpty),
          ]
        : store.groups;
    return [
      for (final g in ordered)
        if (!cfg.hiddenGroups.contains(g.key) &&
            !(cfg.hideEmptyGroups && g.group.count == 0 && g.key.isNotEmpty))
          BoardColumn(
            id: g.key,
            title: label(g) ?? 'No ${def.name}',
            keyLabel: label(g) ?? 'none',
            count: g.group.count,
            color: colors.tone(g.group.color),
            background:
                cfg.colorColumns &&
                    g.key.isNotEmpty &&
                    def.kind != PropKind.checkbox
                ? colors.groupTint(g.group.color)
                : colors.bgSecondary,
          ),
    ];
  }

  BoardCard _card(PageRow row) => BoardCard(id: row.id, title: row.title);

  @override
  int cardCount(String columnId) => group(columnId).rows.total;

  @override
  BoardCard? cardAt(String columnId, int index) {
    final row = group(columnId).rows.rowAt(index);
    return row == null ? null : _card(row);
  }

  @override
  Iterable<BoardCard> loadedCards(String columnId) =>
      group(columnId).rows.loadedRows.map(_card);

  PageRow row(String columnId, String cardId) =>
      group(columnId).rows.loadedRows.firstWhere((r) => r.id == cardId);

  @override
  Future<void> moveCard(
    String cardId,
    String fromColumnId,
    String toColumnId,
    String? afterCardId,
  ) async {
    if (afterCardId == cardId) return;
    final to = group(toColumnId);
    if (fromColumnId == toColumnId) {
      await args.store.moveRow(cardId, afterCardId, within: to.rows);
    } else {
      await args.store.moveToGroup(
        cardId,
        group(fromColumnId),
        to,
        afterCardId,
      );
    }
  }

  @override
  Future<void> addCard(BuildContext context, String columnId) async {
    final g = group(columnId);
    final def = args.store.groupDef!;
    final value = g.key.isEmpty
        ? Values.empty
        : switch (def.kind) {
            PropKind.checkbox => Values.checkbox(g.key == 'true'),
            PropKind.multiSelect => Values.multi([g.key]),
            _ => g.group.value,
          };
    final id = await args.ctl.newRow(
      context,
      values: {if (!value.isEmpty) def.key: value},
      groupKey: g.key,
      open: false,
    );
    args.ctl.editTitleOf = id;
    if (!_disposed) notifyListeners();
  }

  @override
  void openCard(BuildContext context, BoardCard card) =>
      args.ctl.openRow(context, card.id, title: card.title);
}
