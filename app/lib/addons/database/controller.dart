// What views ask of the database around them: open a row (a sheet on the
// desktop stack, a full page on mobile), edit a value (through
// the property registry), add a row, open a property's menu.

import 'package:flutter/scheduler.dart';
import 'package:flutter/widgets.dart';

import '../../core/models.dart';
import '../../services/legacy_host.dart';
import '../../registry/property_registry.dart';
import '../../registry/view_registry.dart';
import '../../state/collection_store.dart';
import '../../ui/ui.dart';
import 'editors/text_editor.dart';
import 'property_menu.dart';
import '../../ui/database/values.dart';

class DbController implements ViewController {
  DbController(this.store);
  @override
  final CollectionStore store;

  /// A row whose title should open for editing when its cell is built (a
  /// row just added from the table).
  @override
  String? editTitleOf;

  @override
  ViewDef? get view => store.view;

  /// Opens row [id] as a page: a sheet on top of the desktop stack, a full
  /// page on mobile. The view's "Open pages in" setting is kept in its
  /// config for compatibility but no longer changes this. A [isNew] row
  /// opens with its title focused, as a new page does.
  @override
  Future<void> openRow(
    BuildContext context,
    String id, {
    String title = '',
    bool isNew = false,
  }) => openPage(
    context,
    id,
    title,
    isNew: isNew,
    stack: true,
    preserveMobileStack: true,
  );

  /// Edits [def] of [row] from the cell [cell]: a checkbox toggles, other
  /// types open their editor.
  @override
  Future<void> edit(BuildContext cell, PageRow row, PropDef def) async {
    final t = propertyRegistry.of(def.kind);
    if (t == null) return;
    final v = row.valueOf(def);
    final tap = t.tap;
    if (tap != null) {
      await store.setValue(row.id, def.key, tap(v));
      return;
    }
    final e = t.edit;
    if (e == null) return;
    await e(
      EditArgs(
        context: cell,
        def: def,
        value: v,
        anchor: globalRectOf(cell),
        onChanged: (nv) => store.setValue(row.id, def.key, nv),
        saveDef: store.updateProperty,
        collection: store.collection,
      ),
    );
  }

  /// Edits the title of [row] in a box over [cell].
  @override
  Future<void> editTitle(BuildContext cell, PageRow row) => editAsText(
    EditArgs(
      context: cell,
      def: const PropDef(
        key: BuiltinKeys.title,
        name: 'Name',
        kind: PropKind.text,
        options: [],
        config: '',
      ),
      value: Values.text(row.title),
      anchor: globalRectOf(cell),
      onChanged: (v) => store.renameRow(row.id, v.text),
      saveDef: (d) async => d,
      collection: store.collection,
    ),
    initial: row.title,
    multiline: false,
    parse: (s) => Values.text(s.replaceAll('\n', ' ')),
  );

  /// Adds a row with [values] (in group [groupKey]) and opens it unless
  /// [open] is false. Returns its id.
  @override
  Future<String> newRow(
    BuildContext context, {
    Map<String, PropValue> values = const {},
    String? groupKey,
    bool open = true,
    String title = '',
    bool empty = false,
  }) async {
    final all = {...values};
    // A filtered view: the new row takes the values its simple conditions
    // ask for, so it stays visible.
    final f = view?.filter;
    if (f != null) {
      for (final c in _conds(f)) {
        if (all.containsKey(c.prop) || c.prop.startsWith(r'$')) continue;
        if (c.op == FilterOp.is_ ||
            (c.op == FilterOp.contains &&
                c.value.kind == ValueKind.multiSelect)) {
          all[c.prop] = c.value;
        } else if (c.op == FilterOp.checked) {
          all[c.prop] = Values.checkbox(true);
        }
      }
    }
    final id = await store.addRow(
      values: all,
      groupKey: groupKey,
      title: title,
      empty: empty,
    );
    if (open && context.mounted) {
      await openRow(context, id, isNew: true);
    }
    return id;
  }

  static Iterable<Condition> _conds(Filter f) sync* {
    if (f.kind == FilterKind.cond) {
      if (f.cond != null) yield f.cond!;
    } else if (f.kind == FilterKind.and) {
      for (final c in f.children) {
        yield* _conds(c);
      }
    }
  }

  /// The property menu of [def] (header click).
  @override
  Future<void> propertyMenu(BuildContext header, PropDef? def) =>
      showPropertyMenu(header, this, def);

  /// "+" at the end of the header: picks a type and adds a property.
  @override
  Future<void> addProperty(BuildContext anchor, {int? index}) async {
    final kind = await pickPropertyType(anchor, globalRectOf(anchor));
    if (kind == null) return;
    final key = await store.addProperty(kind, inView: view, index: index);
    if (!anchor.mounted) return;
    final d = store.def(key);
    if (d != null) {
      SchedulerBinding.instance.addPostFrameCallback((_) {
        if (anchor.mounted) showPropertyMenu(anchor, this, d, editing: true);
      });
    }
  }
}
