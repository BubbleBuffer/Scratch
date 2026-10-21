// The cover picker: a "Gallery" of colours and gradients (rendered to a
// PNG and stored as an attachment, since a cover is an attachment id) and
// "Upload" (a file from the system dialog, or a typed path, copied into the
// vault). "Remove" clears the cover.

import 'dart:ui' as ui;

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../core/file_dialogs.dart';
import '../../editor/block_pickers.dart' show addPickedAttachment;
import '../../state/app_state.dart';
import '../../ui/ui.dart';

/// Gallery entries: (name, colours). One colour is a flat fill; more make a
/// diagonal gradient.
const coverGallery = <(String, List<Color>)>[
  ('Red', [Color(0xFFE16259)]),
  ('Yellow', [Color(0xFFDFAB01)]),
  ('Blue', [Color(0xFF0B6E99)]),
  ('Wood', [Color(0xFFB3906F)]),
  ('Gradient 1', [Color(0xFFFF4E9B), Color(0xFFFFE5B4), Color(0xFF9EF0D8), Color(0xFF7F9CF5)]),
  ('Gradient 2', [Color(0xFF3B82F6), Color(0xFF9333EA)]),
  ('Gradient 3', [Color(0xFFF59E0B), Color(0xFFEF4444)]),
  ('Gradient 4', [Color(0xFF10B981), Color(0xFF3B82F6)]),
  ('Gradient 5', [Color(0xFFFDE68A), Color(0xFFFCA5A5)]),
  ('Gradient 6', [Color(0xFF111827), Color(0xFF6B7280)]),
];

Gradient _gradient(List<Color> colors) => LinearGradient(
      begin: Alignment.topLeft,
      end: Alignment.bottomRight,
      colors: colors.length == 1 ? [colors[0], colors[0]] : colors,
    );

/// [colors] rendered to a [w]x[h] PNG.
Future<List<int>> renderCoverPng(List<Color> colors, {int w = 1500, int h = 600}) async {
  final rec = ui.PictureRecorder();
  final canvas = Canvas(rec);
  final rect = Rect.fromLTWH(0, 0, w.toDouble(), h.toDouble());
  canvas.drawRect(rect, Paint()..shader = _gradient(colors).createShader(rect));
  final img = await rec.endRecording().toImage(w, h);
  final data = await img.toByteData(format: ui.ImageByteFormat.png);
  img.dispose();
  return data!.buffer.asUint8List();
}

/// Completes with a cover attachment id, '' to remove, or null.
Future<String?> showCoverPicker(BuildContext context, {required String pageId, Rect? anchor, bool hasCover = true}) {
  Widget body(BuildContext ctx) => _CoverPicker(hasCover: hasCover, onDone: (v) => Navigator.of(ctx).pop(v));
  if (context.mobile || anchor == null) {
    return showSheet<String>(context, (ctx) => SheetSurface(children: [body(ctx)]));
  }
  return showPopover<String>(context, anchor, (ctx) => MenuSurface(width: 540, child: body(ctx)));
}

class _CoverPicker extends StatefulWidget {
  const _CoverPicker({required this.hasCover, required this.onDone});
  final bool hasCover;
  final ValueChanged<String> onDone;

  @override
  State<_CoverPicker> createState() => _CoverPickerState();
}

class _CoverPickerState extends State<_CoverPicker> {
  int _tab = 0;
  bool _busy = false;
  String? _error;
  final _path = TextEditingController();
  final _focus = FocusNode();

  @override
  void dispose() {
    _path.dispose();
    _focus.dispose();
    super.dispose();
  }

  Future<void> _pickGallery(int i) async {
    if (_busy) return;
    setState(() => _busy = true);
    final vault = context.app.vault;
    final (name, colors) = coverGallery[i];
    final png = await renderCoverPng(colors);
    final id = await vault.run(
        (v) => v.addAttachmentBytes(name: 'cover-${name.toLowerCase().replaceAll(' ', '-')}.png', bytes: png),
        writes: true);
    if (mounted) widget.onDone(id);
  }

  /// Picks an image in the system dialog.
  Future<void> _choose() async {
    if (_busy) return;
    final vault = context.app.vault;
    final files = await FileDialogs.instance.pickFiles(title: 'Upload cover', images: true);
    if (files.isEmpty || !mounted) return;
    setState(() => _busy = true);
    try {
      final id = await addPickedAttachment(vault, files.first);
      if (mounted) widget.onDone(id);
    } catch (_) {
      if (mounted) {
        setState(() {
          _busy = false;
          _error = 'Could not add the file.';
        });
      }
    }
  }

  Future<void> _upload() async {
    final p = _path.text.trim().replaceAll('"', '');
    if (p.isEmpty) {
      setState(() => _error = 'Enter a file path.');
      return;
    }
    final vault = context.app.vault;
    setState(() => _busy = true);
    try {
      final id = await vault.run((v) => v.addAttachmentFromPath(path: p), writes: true);
      if (mounted) widget.onDone(id);
    } catch (_) {
      if (mounted) {
        setState(() {
          _busy = false;
          _error = 'Could not add the file.';
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final dialogs = FileDialogs.instance.available;
    final pathField = Container(
      height: 32,
      padding: const EdgeInsets.symmetric(horizontal: 8),
      alignment: Alignment.centerLeft,
      decoration: BoxDecoration(
        color: c.bgHover,
        borderRadius: BorderRadius.circular(Radii.md),
        border: Border.all(color: c.border),
      ),
      child: PlainField(
        key: const ValueKey('cover-path'),
        controller: _path,
        focusNode: _focus,
        autofocus: !dialogs,
        style: ts(ty.ui, c.textPrimary, weight: 400),
        placeholder: dialogs ? 'Or paste a path to an image…' : 'Path to an image file…',
        textInputAction: TextInputAction.done,
        onSubmitted: (_) => _upload(),
      ),
    );
    Widget tab(int i, String label) => Hover(
          key: ValueKey('cover-tab-$i'),
          onTap: () => setState(() => _tab = i),
          radius: 0,
          child: Container(
            height: 40,
            padding: const EdgeInsets.symmetric(horizontal: 8),
            alignment: Alignment.center,
            decoration: BoxDecoration(
              border: Border(bottom: BorderSide(color: _tab == i ? c.textPrimary : const Color(0x00000000), width: 2)),
            ),
            child: Text(label, style: ts(ty.ui, _tab == i ? c.textPrimary : c.textSecondary)),
          ),
        );
    final content = _tab == 0
        ? Padding(
            padding: const EdgeInsets.all(12),
            child: Column(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [
              Text('Color & Gradient', style: ts(ty.uiSmall, c.textSecondary)),
              const SizedBox(height: 8),
              Wrap(spacing: 8, runSpacing: 8, children: [
                for (var i = 0; i < coverGallery.length; i++)
                  Hover(
                    key: ValueKey('cover-gallery-$i'),
                    onTap: () => _pickGallery(i),
                    radius: Radii.sm,
                    child: Container(
                      width: 116,
                      height: 64,
                      decoration: BoxDecoration(
                        gradient: _gradient(coverGallery[i].$2),
                        borderRadius: BorderRadius.circular(Radii.sm),
                      ),
                    ),
                  ),
              ]),
            ]),
          )
        : Padding(
            padding: const EdgeInsets.all(12),
            child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, mainAxisSize: MainAxisSize.min, children: [
              if (dialogs) ...[
                Center(child: SizedBox(width: 240, child: PrimaryButton(key: const ValueKey('cover-upload'), label: 'Upload file', onTap: _choose))),
                const SizedBox(height: 10),
                pathField,
              ] else ...[
                pathField,
                const SizedBox(height: 10),
                Center(child: SizedBox(width: 240, child: PrimaryButton(key: const ValueKey('cover-upload'), label: 'Upload file', onTap: _upload))),
              ],
              const SizedBox(height: 8),
              Text(_error ?? 'Images wider than 1500 pixels work best.',
                  textAlign: TextAlign.center,
                  style: ts(ty.uiSmall, _error == null ? c.textTertiary : c.inlineCodeText, weight: 400)),
            ]),
          );
    return Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Container(
        padding: const EdgeInsets.symmetric(horizontal: 8),
        decoration: BoxDecoration(border: Border(bottom: BorderSide(color: c.border))),
        child: Row(children: [
          tab(0, 'Gallery'),
          tab(1, 'Upload'),
          const Spacer(),
          if (widget.hasCover)
            TextBtn('Remove', key: const ValueKey('cover-picker-remove'), onTap: () => widget.onDone(''), color: c.textSecondary),
        ]),
      ),
      content,
    ]);
  }
}
