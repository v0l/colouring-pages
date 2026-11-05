package io.v0l.colouringpages

import android.app.NativeActivity
import android.content.ContentValues
import android.content.Intent
import android.os.Bundle
import android.os.Environment
import android.os.ParcelFileDescriptor
import android.print.PageRange
import android.print.PrintAttributes
import android.print.PrintDocumentAdapter
import android.print.PrintDocumentInfo
import android.print.PrintManager
import android.os.CancellationSignal
import android.provider.MediaStore
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream

class MainActivity : NativeActivity() {
    private var pickTarget: File? = null

    external fun pictureChosen(chosen: Boolean)

    fun pickPicture(path: String) {
        pickTarget = File(path)
        runOnUiThread {
            val intent = Intent(Intent.ACTION_GET_CONTENT).apply {
                type = "image/*"
                addCategory(Intent.CATEGORY_OPENABLE)
            }
            try {
                startActivityForResult(Intent.createChooser(intent, "Pick a picture to draw from"), PICK_PICTURE)
            } catch (e: Exception) {
                pictureChosen(false)
            }
        }
    }

    @Deprecated("Deprecated in Java")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != PICK_PICTURE) return
        val uri = data?.data
        val target = pickTarget
        if (resultCode != RESULT_OK || uri == null || target == null) {
            pictureChosen(false)
            return
        }
        Thread {
            val copied = try {
                contentResolver.openInputStream(uri)!!.use { input ->
                    FileOutputStream(target).use { input.copyTo(it) }
                }
                true
            } catch (e: Exception) {
                false
            }
            pictureChosen(copied)
        }.start()
    }

    fun printPdf(path: String) {
        runOnUiThread {
            val manager = getSystemService(PRINT_SERVICE) as PrintManager
            val file = File(path)
            manager.print("Colouring pages", object : PrintDocumentAdapter() {
                override fun onLayout(
                    old: PrintAttributes?,
                    attrs: PrintAttributes?,
                    cancel: CancellationSignal?,
                    callback: LayoutResultCallback,
                    extras: Bundle?,
                ) {
                    val info = PrintDocumentInfo.Builder(file.name)
                        .setContentType(PrintDocumentInfo.CONTENT_TYPE_DOCUMENT)
                        .build()
                    callback.onLayoutFinished(info, true)
                }

                override fun onWrite(
                    pages: Array<PageRange>,
                    destination: ParcelFileDescriptor,
                    cancel: CancellationSignal?,
                    callback: WriteResultCallback,
                ) {
                    try {
                        FileInputStream(file).use { input ->
                            FileOutputStream(destination.fileDescriptor).use { out -> input.copyTo(out) }
                        }
                        callback.onWriteFinished(arrayOf(PageRange.ALL_PAGES))
                    } catch (e: Exception) {
                        callback.onWriteFailed(e.message)
                    }
                }
            }, null)
        }
    }

    fun saveDownload(path: String, name: String, mime: String): Boolean {
        return try {
            val values = ContentValues().apply {
                put(MediaStore.MediaColumns.DISPLAY_NAME, name)
                put(MediaStore.MediaColumns.MIME_TYPE, mime)
                put(MediaStore.MediaColumns.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS + "/colouring-pages")
            }
            val uri = contentResolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values) ?: return false
            contentResolver.openOutputStream(uri)!!.use { out ->
                FileInputStream(File(path)).use { it.copyTo(out) }
            }
            true
        } catch (e: Exception) {
            false
        }
    }

    companion object {
        const val PICK_PICTURE = 4185

        init {
            System.loadLibrary("colouring_pages")
        }
    }
}
