package dev.notesapp.tauri

import android.os.Bundle
import android.view.View
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // Android 15 draws apps edge to edge and no longer resizes them for the
    // keyboard. Pad the content by the system bars and the keyboard instead,
    // so the webview shrinks when the keyboard opens and the page's toolbar
    // can sit right above it.
    val content = findViewById<View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { v, insets ->
      val i = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.ime() or WindowInsetsCompat.Type.displayCutout()
      )
      v.setPadding(i.left, i.top, i.right, i.bottom)
      WindowInsetsCompat.CONSUMED
    }
  }
}
