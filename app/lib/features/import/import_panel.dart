// Markdown import and export, with the system's file dialogs ("Choose…")
// and a typed path as the fallback.
// - Import: a folder, then import_markdown (into a parent page or at the
//   top level) with a spinner and a result line. Where folders cannot be
//   read by path (Android), Choose picks .md files, which are copied to a
//   temporary folder and imported from there.
// - Export: the page through the core Markdown renderer, including front
//   matter, written to a file: one chosen in the save
//   dialog, or a typed path, by default Documents/<title>.md.

import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../core/file_dialogs.dart';
import '../../core/models.dart';
import '../../core/vault.dart';
import '../../state/app_state.dart';
import '../../ui/icon_x.dart';
import '../../ui/ui.dart';

/// Imports a markdown folder (into [parent], or at the top level).
Future<void> showImport(BuildContext context, {String? parent}) => _show(
      context,
      (ctx) => ImportPanel(key: const ValueKey('import-panel'), parent: parent),
    );

/// Exports page [pageId] as markdown.
Future<void> showExport(BuildContext context, String pageId) => _show(
      context,
      (ctx) => ExportPanel(key: const ValueKey('export-panel'), pageId: pageId),
    );

Future<void> _show(BuildContext context, WidgetBuilder body) async {
  if (context.mobile) {
    await showSheet<void>(context, (ctx) => SheetSurface(children: [body(ctx)]));
    return;
  }
  await showModalPanel<void>(context, body, width: 480, shrink: true);
}

/// [name] with characters that file systems refuse replaced by `_`.
String safeFileName(String name) => name.replaceAll(RegExp(r'[<>:"/\\|?*\x00-\x1F]'), '_');

/// The default export file: Documents/<title>.md in the user's home.
String defaultExportPath(String title) {
  final env = Platform.environment;
  final home = env['USERPROFILE'] ?? env['HOME'] ?? Directory.systemTemp.path;
  var name = safeFileName(displayTitle(title.trim()));
  if (name.length > 80) name = name.substring(0, 80);
  final sep = Platform.pathSeparator;
  return '$home${sep}Documents$sep$name.md';
}

/// The page through the core vault writer's renderer, including type data.
Future<String> pageToMarkdown(Vault vault, String pageId) => vault.exportMarkdown(pageId);

/// The frame both dialogs share: header with an icon, a hint, a path field,
/// the action button and a status line.
class _PathDialog extends StatelessWidget {
  const _PathDialog({
    required this.icon,
    required this.title,
    required this.hint,
    required this.field,
    required this.button,
    required this.status,
  });
  final Ix icon;
  final String title, hint;
  final Widget field, button, status;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final m = context.mobile;
    final pad = m ? Layout.gutter : 20.0;
    return Padding(
      padding: EdgeInsets.fromLTRB(pad, m ? 0 : 18, pad, m ? 8 : 18),
      child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        Row(children: [
          IconX(icon, size: 20, color: c.iconSecondary),
          const SizedBox(width: 8),
          Text(title, style: tsDisplay(const TypeSpec(19, 24, 640), c.textPrimary)),
        ]),
        const SizedBox(height: 6),
        Text(hint, style: ts(ty.uiSmall, c.textSecondary, weight: 400)),
        const SizedBox(height: 14),
        field,
        const SizedBox(height: 12),
        button,
        const SizedBox(height: 10),
        status,
      ]),
    );
  }
}

Widget _pathField(BuildContext context, Key key, TextEditingController ctl, FocusNode focus, String placeholder,
    VoidCallback onSubmit, {Key? chooseKey, VoidCallback? onChoose}) {
  final c = context.colors;
  final field = InputBox(
    child: PlainField(
      key: key,
      controller: ctl,
      focusNode: focus,
      autofocus: !context.mobile,
      style: ts(context.type.ui, c.textPrimary, weight: 400),
      placeholder: placeholder,
      textInputAction: TextInputAction.done,
      onSubmitted: (_) => onSubmit(),
    ),
  );
  if (onChoose == null) return field;
  return Row(children: [
    Expanded(child: field),
    const SizedBox(width: 8),
    DialogButton(key: chooseKey, label: 'Choose…', onTap: onChoose),
  ]);
}

Widget _status(BuildContext context, Key key, bool busy, String? text, bool error, String busyText) {
  final c = context.colors;
  final style = ts(context.type.uiSmall, error ? c.inlineCodeText : c.textSecondary, weight: 400);
  if (busy) {
    return Row(children: [
      const Spinner(size: 14),
      const SizedBox(width: 8),
      Expanded(child: Text(busyText, style: style)),
    ]);
  }
  return Text(text ?? '', key: key, style: style);
}

class ImportPanel extends StatefulWidget {
  const ImportPanel({super.key, this.parent});
  final String? parent;

  @override
  State<ImportPanel> createState() => _ImportPanelState();
}

class _ImportPanelState extends State<ImportPanel> {
  final _ctl = TextEditingController();
  final _focus = FocusNode();
  bool _busy = false, _error = false;
  String? _result;

  @override
  void dispose() {
    _ctl.dispose();
    _focus.dispose();
    super.dispose();
  }

  /// Picks a folder (or, where folders cannot be read, Markdown files) in
  /// the system dialog and imports it.
  Future<void> _choose() async {
    if (_busy) return;
    final d = FileDialogs.instance;
    if (d.folders) {
      final dir = await d.pickFolder(title: 'Import a folder of Markdown files');
      if (dir == null || !mounted) return;
      _ctl.text = dir;
      return _run();
    }
    final files = await d.pickFiles(title: 'Import Markdown files', multiple: true, extensions: const ['md', 'markdown']);
    if (files.isEmpty || !mounted) return;
    setState(() {
      _busy = true;
      _result = null;
    });
    Directory? tmp;
    try {
      tmp = await Directory.systemTemp.createTemp('notes_import_');
      for (final f in files) {
        final name = safeFileName(f.name);
        await File('${tmp.path}${Platform.pathSeparator}$name').writeAsBytes(await f.bytes());
      }
    } catch (e) {
      if (mounted) {
        setState(() {
          _busy = false;
          _error = true;
          _result = 'Could not read the files.';
        });
      }
      return;
    }
    if (!mounted) return;
    setState(() => _busy = false);
    await _run(folder: tmp.path);
    try {
      await tmp.delete(recursive: true);
    } catch (_) {}
  }

  Future<void> _run({String? folder}) async {
    if (_busy) return;
    final dir = folder ?? _ctl.text.trim().replaceAll('"', '');
    if (dir.isEmpty) {
      setState(() {
        _error = true;
        _result = 'Enter the path of a folder.';
      });
      return;
    }
    if (!Directory(dir).existsSync()) {
      setState(() {
        _error = true;
        _result = 'That folder does not exist.';
      });
      return;
    }
    setState(() {
      _busy = true;
      _result = null;
    });
    try {
      final r = await context.app.vault.run((v) => v.importMarkdown(dir: dir, parent: widget.parent), writes: true);
      if (!mounted) return;
      final parts = [
        'Imported ${r.pages} ${r.pages == 1 ? 'page' : 'pages'}',
        if (r.attachments > 0) '${r.attachments} ${r.attachments == 1 ? 'attachment' : 'attachments'}',
        if (r.unresolvedLinks > 0) '${r.unresolvedLinks} unresolved ${r.unresolvedLinks == 1 ? 'link' : 'links'}',
      ];
      setState(() {
        _busy = false;
        _error = false;
        _result = '${parts.join(', ')}.';
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _busy = false;
        _error = true;
        _result = 'Import failed: $e';
      });
    }
  }

  @override
  Widget build(BuildContext context) => _PathDialog(
        icon: Ix.import,
        title: 'Import',
        hint: FileDialogs.instance.available && !FileDialogs.instance.folders
            // Choose picks files here; a typed folder path still works.
            ? (widget.parent == null
                ? 'Import Markdown files (for example from an Obsidian vault) as pages.'
                : 'Import Markdown files as pages inside this page.')
            : (widget.parent == null
                ? 'Import a folder of Markdown files (for example an Obsidian vault) as pages.'
                : 'Import a folder of Markdown files as pages inside this page.'),
        field: _pathField(context, const ValueKey('import-path'), _ctl, _focus, 'Path to a folder…', _run,
            chooseKey: const ValueKey('import-choose'),
            onChoose: FileDialogs.instance.available && !_busy ? _choose : null),
        button: DialogButton(key: const ValueKey('import-run'), label: 'Import', filled: true, onTap: _busy ? null : _run),
        status: _status(context, const ValueKey('import-result'), _busy, _result, _error, 'Importing…'),
      );
}

class ExportPanel extends StatefulWidget {
  const ExportPanel({super.key, required this.pageId});
  final String pageId;

  @override
  State<ExportPanel> createState() => _ExportPanelState();
}

class _ExportPanelState extends State<ExportPanel> {
  final _ctl = TextEditingController();
  final _focus = FocusNode();
  bool _busy = false, _error = false;
  String? _result;
  PageData? _page;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    try {
      final p = await context.app.vault.loadPage(widget.pageId);
      if (!mounted) return;
      setState(() {
        _page = p;
        if (_ctl.text.isEmpty) _ctl.text = defaultExportPath(p.title);
      });
    } catch (e) {
      if (mounted) {
        setState(() {
          _error = true;
          _result = 'Could not load the page.';
        });
      }
    }
  }

  @override
  void dispose() {
    _ctl.dispose();
    _focus.dispose();
    super.dispose();
  }

  /// Asks where to save in the system dialog and writes the file there.
  Future<void> _choose() async {
    final p = _page;
    if (_busy || p == null) return;
    final name = File(defaultExportPath(p.title)).uri.pathSegments.last;
    setState(() => _busy = true);
    try {
      final markdown = await pageToMarkdown(context.app.vault, widget.pageId);
      final where = await FileDialogs.instance
          .saveFile(name: name, bytes: utf8.encode(markdown), title: 'Export as Markdown', extension: 'md');
      if (!mounted) return;
      setState(() {
        _busy = false;
        if (where != null) {
          _error = false;
          _result = 'Exported to $where';
          if (File(where).isAbsolute) _ctl.text = where;
        }
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _busy = false;
        _error = true;
        _result = 'Export failed: $e';
      });
    }
  }

  Future<void> _run() async {
    final p = _page;
    if (_busy || p == null) return;
    final path = _ctl.text.trim().replaceAll('"', '');
    if (path.isEmpty) {
      setState(() {
        _error = true;
        _result = 'Enter a file path.';
      });
      return;
    }
    setState(() => _busy = true);
    try {
      final markdown = await pageToMarkdown(context.app.vault, widget.pageId);
      final f = File(path);
      await f.parent.create(recursive: true);
      await f.writeAsString(markdown);
      if (!mounted) return;
      setState(() {
        _busy = false;
        _error = false;
        _result = 'Exported to $path';
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _busy = false;
        _error = true;
        _result = 'Export failed: $e';
      });
    }
  }

  @override
  Widget build(BuildContext context) => _PathDialog(
        icon: Ix.export,
        title: 'Export as Markdown',
        hint: 'Writes this page as Markdown with its metadata and body.',
        field: _pathField(context, const ValueKey('export-path'), _ctl, _focus, 'Path to a .md file…', _run,
            chooseKey: const ValueKey('export-choose'),
            onChoose: FileDialogs.instance.available && !_busy && _page != null ? _choose : null),
        button: DialogButton(
            key: const ValueKey('export-run'), label: 'Export', filled: true, onTap: _busy || _page == null ? null : _run),
        status: _status(context, const ValueKey('export-result'), _busy, _result, _error, 'Exporting…'),
      );
}
