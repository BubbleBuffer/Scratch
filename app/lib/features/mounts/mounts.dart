// Folder mounts (desktop): a folder of this computer shown as a page whose
// children are the folder's Markdown files. Settings → Vault lists them; a
// path field adds one, "Remove" unmounts (the pages stay in the vault, the
// folder's files are left alone). The core reads outside changes every two
// seconds (the bridge's poll thread); see wiki/architecture/mounts.md.

import 'dart:io';

import 'package:flutter/widgets.dart';

import '../../src/rust/api/types.dart';
import '../../state/app_state.dart';
import '../../ui/app_name.dart';
import '../../ui/ui.dart';
import '../settings/controls.dart';

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
      _mounts = vault.raw.listMounts();
    });
  }

  Future<void> _remove(MountInfo m) async {
    final vault = context.app.vault;
    await vault.run((v) => v.removeMount(page: m.page), writes: true);
    if (mounted) setState(() => _mounts = vault.raw.listMounts());
  }

  static String _message(Object e) => 'Could not mount it: $e';

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      const SettingsHeading('Mounted folders', top: 24),
      SettingsRow(
        label: 'Mount a folder',
        description: 'Its Markdown files become pages, and edits are saved back to the files. '
            'Changes made outside $appName show up within a few seconds.',
        stack: true,
        trailing: Row(children: [
          Expanded(
            child: SettingsField(
              fieldKey: const ValueKey('mount-path-field'),
              controller: _path,
              placeholder: Platform.isWindows ? r'C:\Users\me\Notes' : '/home/me/notes',
              onSubmitted: (_) => _add(),
            ),
          ),
          const SizedBox(width: 8),
          OutlineButton(key: const ValueKey('mount-add'), label: 'Mount', onTap: _add),
        ]),
      ),
      if (_error != null)
        Text(_error!, key: const ValueKey('mount-error'), style: tsRaw(13, 18, 400, c.inlineCodeText)),
      for (final (i, m) in _mounts.indexed)
        SettingsRow(
          key: ValueKey('mount-$i'),
          label: m.folder.split(RegExp(r'[\\/]')).lastWhere((s) => s.isNotEmpty, orElse: () => m.folder),
          description: m.folder,
          trailing: OutlineButton(key: ValueKey('mount-remove-$i'), label: 'Remove', onTap: () => _remove(m)),
        ),
    ]);
  }
}
