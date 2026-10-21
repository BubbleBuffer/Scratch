// Files & media: attachments in the vault's blob store, shown as file
// chips (paperclip and name). The editor lists them, removes them and adds
// new ones through the same picker as file blocks.

import 'package:flutter/widgets.dart';

import '../../../core/models.dart';
import '../../../core/vault.dart';
import '../../../services/legacy_host.dart';
import '../cells.dart';
import '../../../ui/database/db_icons.dart';
import '../../../ui/database/values.dart';
import '../../../ui/database/widgets.dart';
import '../../../registry/property_registry.dart';
import '../../../state/app_state.dart';
import '../../../ui/ui.dart';

/// Attachment names by id, loaded once.
class AttachmentNames {
  static final _cache = <String, String>{};
  static final _loading = <String>{};
  static final changed = ValueNotifier<int>(0);

  static String of(String id) => _cache[id] ?? '';

  static void load(Vault vault, Iterable<String> ids) {
    for (final id in ids) {
      if (_cache.containsKey(id) || !_loading.add(id)) continue;
      vault.run((v) => v.attachmentInfo(id: id)).then((a) {
        _cache[id] = a.name;
        changed.value++;
      }, onError: (Object _) {
        _cache[id] = 'Missing file';
        changed.value++;
      }).whenComplete(() => _loading.remove(id));
    }
  }
}

final filesProperty = PropertyType(
  kind: PropKind.files,
  label: 'Files & media',
  icon: DbIco.files,
  format: (v, d) => [for (final id in v.ids) AttachmentNames.of(id)].join(', '),
  cell: (a) => a.value.ids.isEmpty ? emptyCell(a) : FilesCell(ids: a.value.ids, mode: a.mode, wrap: a.wrap),
  edit: (a) => showPanel<void>(a.context, a.anchor, (ctx) => _FilesEditor(args: a), width: 300),
  filterOps: const [FilterOp.isEmpty, FilterOp.isNotEmpty],
);

/// File chips: a paperclip and the file name on a grey pill.
class FilesCell extends StatelessWidget {
  const FilesCell({super.key, required this.ids, required this.mode, this.wrap = false});
  final List<String> ids;
  final CellMode mode;
  final bool wrap;

  @override
  Widget build(BuildContext context) {
    AttachmentNames.load(context.app.vault, ids);
    return ValueListenableBuilder<int>(
      valueListenable: AttachmentNames.changed,
      builder: (context, _, _) {
        final tag = tagStyle(context, 'default');
        final small = mode == CellMode.card || mode == CellMode.list;
        final style = cellStyle(context, mode).copyWith(fontSize: small ? 12 : 14, color: tag.text);
        final chips = [
          for (final id in ids)
            Container(
              key: ValueKey('file-chip-$id'),
              height: small ? 18 : 20,
              padding: const EdgeInsets.symmetric(horizontal: 6),
              decoration: BoxDecoration(color: tag.bg, borderRadius: BorderRadius.circular(Radii.sm)),
              child: Row(mainAxisSize: MainAxisSize.min, children: [
                DbIcon(DbIco.files, size: small ? 12 : 14, color: context.colors.iconSecondary),
                const SizedBox(width: 4),
                Flexible(
                  child: Text(AttachmentNames.of(id),
                      maxLines: 1, overflow: TextOverflow.ellipsis, style: style.copyWith(height: 1.2)),
                ),
              ]),
            ),
        ];
        if (wrap || mode == CellMode.page || mode == CellMode.card) {
          return Wrap(spacing: 6, runSpacing: 6, children: chips);
        }
        return ClipRect(
          child: OverflowBox(
            alignment: Alignment.centerLeft,
            maxWidth: double.infinity,
            child: Row(mainAxisSize: MainAxisSize.min, children: [
              for (var i = 0; i < chips.length; i++) ...[if (i > 0) const SizedBox(width: 6), chips[i]],
            ]),
          ),
        );
      },
    );
  }
}

class _FilesEditor extends StatefulWidget {
  const _FilesEditor({required this.args});
  final EditArgs args;

  @override
  State<_FilesEditor> createState() => _FilesEditorState();
}

class _FilesEditorState extends State<_FilesEditor> {
  late List<String> ids = [...widget.args.value.ids];

  void _set(List<String> next) {
    setState(() => ids = next);
    widget.args.onChanged(Values.files(next));
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    AttachmentNames.load(context.app.vault, ids);
    return ValueListenableBuilder<int>(
      valueListenable: AttachmentNames.changed,
      builder: (context, _, _) => Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        for (final id in ids)
          MenuItem(
            key: ValueKey('file-item-$id'),
            label: AttachmentNames.of(id),
            leading: DbIcon(DbIco.files, size: 18, color: c.iconSecondary),
            trailing: Icon20(Ico.close, size: 14, color: c.iconTertiary),
            onTap: () => _set([...ids]..remove(id)),
          ),
        if (ids.isNotEmpty) const MenuSeparator(),
        Builder(
          builder: (b) => MenuItem(
            key: const ValueKey('file-add'),
            label: 'Add a file or image',
            icon: Ico.plus,
            onTap: () async {
              final id = await pickAttachment(b, globalRectOf(b), image: false);
              if (id == null || !mounted || ids.contains(id)) return;
              _set([...ids, id]);
            },
          ),
        ),
      ]),
    );
  }
}
