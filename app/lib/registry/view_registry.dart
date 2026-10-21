// The view registry. Each database view type (table, board, list,
// calendar, gallery, timeline) is registered by its add-on (lib/addons/) as a
// [ViewType]. The database frame picks the widget for a view, the layout
// picker lists the layouts, and view settings add each layout's options,
// only through this registry.

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../state/collection_store.dart';
import '../ui/database/db_icons.dart';

/// What a view's body asks of the database around it. The database add-on's
/// controller implements it; the registry and the views see only this.
abstract interface class ViewController {
  CollectionStore get store;
  ViewDef? get view;

  /// A row whose title should open for editing when its cell is built.
  String? get editTitleOf;
  set editTitleOf(String? id);

  /// Opens row [id] as a page.
  Future<void> openRow(
    BuildContext context,
    String id, {
    String title = '',
    bool isNew = false,
  });

  /// Edits [def] of [row] from the cell [cell].
  Future<void> edit(BuildContext cell, PageRow row, PropDef def);

  /// Edits the title of [row] in a box over [cell].
  Future<void> editTitle(BuildContext cell, PageRow row);

  /// Adds a row and opens it unless [open] is false. Returns its id.
  Future<String> newRow(
    BuildContext context, {
    Map<String, PropValue> values = const {},
    String? groupKey,
    bool open = true,
    String title = '',
    bool empty = false,
  });

  /// The property menu of [def] (header click).
  Future<void> propertyMenu(BuildContext header, PropDef? def);

  /// "+" at the end of the header: picks a type and adds a property.
  Future<void> addProperty(BuildContext anchor, {int? index});
}

/// What a view's body gets.
class ViewArgs {
  const ViewArgs({
    required this.store,
    required this.view,
    required this.ctl,
    required this.inline,
    required this.gutter,
  });
  final CollectionStore store;
  final ViewDef view;

  /// Opening rows, editing values, adding rows, property menus.
  final ViewController ctl;

  /// An inline database inside a page: the view takes the height of its
  /// content (with "Load more") instead of filling and scrolling.
  final bool inline;

  /// Horizontal padding before the view's content (the page gutter).
  final double gutter;
}

/// One option of a layout in view settings.
class ViewOption {
  const ViewOption({
    required this.label,
    required this.value,
    required this.onTap,
    this.icon,
  });
  final String label;

  /// Shown at the right (a switch state or a choice).
  final Widget value;
  final DbIco? icon;

  /// [anchor] is the row's global rectangle (for sub-menus).
  final void Function(BuildContext context, Rect anchor) onTap;
}

class ViewType {
  const ViewType({
    required this.kind,
    required this.label,
    required this.icon,
    required this.build,
    this.needsDate = false,
    this.needsGroup = false,
    this.options,
  });

  final ViewKind kind;
  final String label;
  final DbIco icon;

  /// The view's body.
  final Widget Function(BuildContext context, ViewArgs a) build;

  /// Calendar and timeline: shows rows by a date property.
  final bool needsDate;

  /// Board: shows rows in groups.
  final bool needsGroup;

  /// The layout's own options in view settings ("Show vertical lines",
  /// "Card size", ...).
  final List<ViewOption> Function(BuildContext context, ViewArgs a)? options;
}

class ViewRegistry {
  final _types = <ViewKind, ViewType>{};

  /// Adds or replaces a type.
  void register(ViewType t, {ViewKind? before}) {
    _types[t.kind] = t;
    if (before != null && before != t.kind && _types.containsKey(before)) {
      final entries = _types.entries.toList();
      _types.clear();
      for (final entry in entries) {
        if (entry.key == t.kind) continue;
        if (entry.key == before) _types[t.kind] = t;
        _types[entry.key] = entry.value;
      }
    }
  }

  ViewType? of(ViewKind kind) => _types[kind];

  /// Registered layouts in their declared menu order.
  Iterable<ViewType> get all => _types.values;
}

final viewRegistry = ViewRegistry();

/// The label of [k] (from the registry, else the core's name).
String viewLabel(ViewKind k) => viewRegistry.of(k)?.label ?? viewKindLabel(k);
