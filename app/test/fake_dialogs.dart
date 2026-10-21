// File dialogs for tests: they answer with what the test queued and record
// what was asked. [pumpApp] installs a fresh one (desktop behaviour: files
// and folders), so no system dialog ever opens.

import 'dart:io';
import 'dart:typed_data';

import 'package:notes_app/core/file_dialogs.dart';

class FakeFileDialogs extends FileDialogs {
  FakeFileDialogs({this.available = true, this.folders = true});

  @override
  final bool available;
  @override
  final bool folders;

  /// The next answers (null or empty means cancelled).
  String? folder;
  List<PickedFile> files = const [];

  /// Where [saveFile] writes (null: cancelled).
  String? savePath;

  /// What was asked, in order: `folder`, `files`, `save:NAME`.
  final List<String> calls = [];

  @override
  Future<String?> pickFolder({String? title}) async {
    calls.add('folder');
    return folder;
  }

  @override
  Future<List<PickedFile>> pickFiles(
      {String? title, bool multiple = false, bool images = false, List<String>? extensions}) async {
    calls.add('files');
    return multiple ? files : files.take(1).toList();
  }

  @override
  Future<String?> saveFile({required String name, required Uint8List bytes, String? title, String? extension}) async {
    calls.add('save:$name');
    final p = savePath;
    if (p == null) return null;
    await File(p).writeAsBytes(bytes);
    return p;
  }
}

/// A picked file given by its bytes only (as the Android picker may).
PickedFile pickedBytes(String name, List<int> bytes) =>
    PickedFile(name, read: () async => Uint8List.fromList(bytes));
