// Folder mounts (desktop): a folder of this computer shown as a page whose
// children are the folder's Markdown files. Settings > Storage lists them;
// "Mount a folder" opens a path field that adds one, "Remove" unmounts (the
// pages stay in the vault, the folder's files are left alone). The core reads
// outside changes every two seconds (the bridge's poll thread); see
// wiki/core/mounts.md.

import 'dart:io';

import 'package:flutter/widgets.dart';

import '../../src/rust/api/types.dart';
import '../../state/app_state.dart';
import '../../ui/app_name.dart';
import '../settings/settings_kit.dart';

class MountsSection extends StatefulWidget {
  const MountsSection({super.key});

  @override
  State<MountsSection> createState() => _MountsSectionState();
}

class _MountsSectionState extends State<MountsSection> {
  final _path = TextEditingController();
  List<MountInfo> _mounts = const [];
  String? _error;
  bool _busy = false;
  bool _editing = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _mounts = context.app.vault.raw.listMounts();
  }

  @override
  void dispose() {
    _path.dispose();
    super.dispose();
  }

  Future<void> _add() async {
    final p = _path.text.trim();
    if (p.isEmpty || _busy) return;
    final vault = context.app.vault;
    setState(() => _busy = true);
    String? error;
    try {
      await vault.run((v) => v.addMount(folder: p, parent: null), writes: true);
      _path.clear();
    } catch (e) {
      error = _message(e);
    }
    if (!mounted) return;
    setState(() {
      _busy = false;
      _error = error;
      if (error == null) _editing = false;
      _mounts = vault.raw.listMounts();
    });
  }

  void _cancel() => setState(() {
    _editing = false;
    _error = null;
    _path.clear();
  });

  Future<void> _remove(MountInfo m) async {
    final vault = context.app.vault;
    await vault.run((v) => v.removeMount(page: m.page), writes: true);
    if (mounted) setState(() => _mounts = vault.raw.listMounts());
  }

  static String _message(Object e) => 'Could not mount it: $e';

  @override
  Widget build(BuildContext context) {
    final add = SButton(
      'Mount a folder',
      key: const ValueKey('mount-open'),
      icon: Sx.plus,
      onTap: () => setState(() => _editing = true),
    );
    return SSection(
      title: 'Mounted folders',
      intro:
          'Show Markdown folders from elsewhere as pages. Edits save back to the files; outside changes appear within seconds.',
      children: [
        if (_mounts.isNotEmpty)
          Padding(
            padding: const EdgeInsets.only(top: 12),
            child: SList(
              foot: _editing ? null : SListFoot(children: [add]),
              children: [
                for (final (i, m) in _mounts.indexed)
                  KeyedSubtree(
                    key: ValueKey('mount-$i'),
                    child: SItem(
                      icon: Sx.folderSync,
                      name: m.folder
                          .split(RegExp(r'[\\/]'))
                          .lastWhere((s) => s.isNotEmpty, orElse: () => m.folder),
                      sub: m.folder,
                      subMono: true,
                      trailing: [
                        SButton(
                          'Remove',
                          key: ValueKey('mount-remove-$i'),
                          kind: SButtonKind.danger,
                          onTap: () => _remove(m),
                        ),
                      ],
                    ),
                  ),
              ],
            ),
          )
        else if (!_editing)
          Padding(
            padding: const EdgeInsets.only(top: 12),
            child: SEmpty(
              icon: Sx.folderSync,
              title: 'No mounted folders',
              text:
                  'Mount a folder of Markdown files, such as a project’s docs, to read and edit it in $appName.',
              action: add,
            ),
          ),
        if (_editing)
          SEditor(
            title: 'Mount a folder',
            desc:
                'Its Markdown files become pages, and edits are saved back to the files. Changes made outside $appName show up within a few seconds.',
            actions: [
              SButton('Cancel', key: const ValueKey('mount-cancel'), kind: SButtonKind.quiet, onTap: _cancel),
              SButton(
                _busy ? 'Mounting…' : 'Mount',
                key: const ValueKey('mount-add'),
                kind: SButtonKind.primary,
                onTap: _busy ? null : _add,
              ),
            ],
            children: [
              SField(
                label: 'Folder',
                child: STextField(
                  fieldKey: const ValueKey('mount-path-field'),
                  controller: _path,
                  mono: true,
                  placeholder: Platform.isWindows ? r'C:\Users\me\Notes' : '/home/me/notes',
                  onSubmitted: (_) => _add(),
                ),
              ),
              if (_error != null)
                SNotice(_error!, kind: SNoticeKind.bad, textKey: const ValueKey('mount-error')),
            ],
          ),
      ],
    );
  }
}
