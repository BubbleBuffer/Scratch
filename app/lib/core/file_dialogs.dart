// The system's file and folder dialogs (file_picker): open dialogs on
// Windows, Linux and macOS, the system picker on Android. Every place that
// uses one also keeps a typed path field, for when there is no dialog.
//
// Tests replace [FileDialogs.instance]; the native dialogs never open under
// `flutter test`.

import 'dart:io';
import 'dart:typed_data';

import 'package:file_picker/file_picker.dart';

/// A file the user picked: a local path when there is one, else its bytes.
class PickedFile {
  const PickedFile(this.name, {this.path, this.read});
  final String name;
  final String? path;
  final Future<Uint8List> Function()? read;

  Future<Uint8List> bytes() => path != null ? File(path!).readAsBytes() : read!();
}

abstract class FileDialogs {
  const FileDialogs();

  /// The dialogs in use (tests set a fake).
  static FileDialogs instance = const NativeFileDialogs();

  /// Whether files can be picked at all.
  bool get available;

  /// Whether a folder can be picked and then read by path (desktop only:
  /// on Android a picked folder is not readable as a path).
  bool get folders;

  /// A folder, or null if cancelled.
  Future<String?> pickFolder({String? title});

  /// Files ([extensions] without dots, [images] for image files), or an
  /// empty list if cancelled.
  Future<List<PickedFile>> pickFiles({String? title, bool multiple = false, bool images = false, List<String>? extensions});

  /// Asks where to save [bytes] as [name] and writes them. Returns where the
  /// file went (a path, or a name on Android), or null if cancelled.
  Future<String?> saveFile({required String name, required Uint8List bytes, String? title, String? extension});
}

class NativeFileDialogs extends FileDialogs {
  const NativeFileDialogs();

  static bool get _test => Platform.environment.containsKey('FLUTTER_TEST');

  @override
  bool get available =>
      !_test && (Platform.isWindows || Platform.isLinux || Platform.isMacOS || Platform.isAndroid || Platform.isIOS);

  @override
  bool get folders => available && (Platform.isWindows || Platform.isLinux || Platform.isMacOS);

  @override
  Future<String?> pickFolder({String? title}) async {
    if (!folders) return null;
    return FilePicker.getDirectoryPath(dialogTitle: title);
  }

  @override
  Future<List<PickedFile>> pickFiles(
      {String? title, bool multiple = false, bool images = false, List<String>? extensions}) async {
    if (!available) return const [];
    final type = images ? FileType.image : (extensions != null ? FileType.custom : FileType.any);
    final files = multiple
        ? await FilePicker.pickFiles(dialogTitle: title, type: type, allowedExtensions: images ? null : extensions)
        : [?await FilePicker.pickFile(dialogTitle: title, type: type, allowedExtensions: images ? null : extensions)];
    return [for (final f in files) PickedFile(f.name, path: f.path, read: f.readAsBytes)];
  }

  @override
  Future<String?> saveFile({required String name, required Uint8List bytes, String? title, String? extension}) async {
    if (!available) return null;
    final uri = await FilePicker.saveFile(
      fileName: name,
      bytes: bytes,
      dialogTitle: title,
      type: extension != null ? FileType.custom : FileType.any,
      allowedExtensions: extension != null ? [extension] : null,
    );
    if (uri == null) return null;
    if (uri.scheme != 'file') return name;
    final path = uri.toFilePath();
    // Desktop dialogs may only return the chosen path; make sure it is written.
    final f = File(path);
    if (!f.existsSync() || f.lengthSync() != bytes.length) await f.writeAsBytes(bytes);
    return path;
  }
}
