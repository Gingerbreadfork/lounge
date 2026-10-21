package tv.lounge.launcher

import android.service.dreams.DreamService
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.platform.ComposeView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.ViewModelStoreOwner
import androidx.lifecycle.setViewTreeLifecycleOwner
import androidx.lifecycle.setViewTreeViewModelStoreOwner
import androidx.savedstate.SavedStateRegistry
import androidx.savedstate.SavedStateRegistryController
import androidx.savedstate.SavedStateRegistryOwner
import androidx.savedstate.setViewTreeSavedStateRegistryOwner
import tv.lounge.design.LoungeTheme
import tv.lounge.design.LoungeThemeVariant
import tv.lounge.design.LoungeVisuals
import tv.lounge.design.systemReducedMotion
import tv.lounge.feature.home.Screensaver
import tv.lounge.platform.Prefs

/** The system screensaver, drawn with Compose from the same snapshot as the home screen. */
class AmbientDreamService : DreamService(), LifecycleOwner, SavedStateRegistryOwner, ViewModelStoreOwner {
    private val lifecycleRegistry = LifecycleRegistry(this)
    private val savedState = SavedStateRegistryController.create(this)
    override val lifecycle: Lifecycle get() = lifecycleRegistry
    override val savedStateRegistry: SavedStateRegistry get() = savedState.savedStateRegistry
    override val viewModelStore: ViewModelStore = ViewModelStore()

    override fun onCreate() {
        super.onCreate()
        savedState.performRestore(null)
        lifecycleRegistry.currentState = Lifecycle.State.CREATED
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        isInteractive = false
        isFullscreen = true
        val app = application as LoungeApplication
        val prefs = app.prefs
        val view = ComposeView(this).apply {
            setViewTreeLifecycleOwner(this@AmbientDreamService)
            setViewTreeSavedStateRegistryOwner(this@AmbientDreamService)
            setViewTreeViewModelStoreOwner(this@AmbientDreamService)
            setContent {
                val uiPrefs by prefs.all.collectAsState(initial = app.startPrefs ?: Prefs.UiPrefs())
                LoungeTheme(
                    variant = LoungeThemeVariant.valueOf(uiPrefs.theme.name),
                    reducedMotion = uiPrefs.reducedMotion ?: systemReducedMotion(),
                    visuals = LoungeVisuals.Lite,
                ) {
                    Screensaver(app.awaitCore(), prefs)
                }
            }
        }
        setContentView(view)
        lifecycleRegistry.currentState = Lifecycle.State.STARTED
    }

    override fun onDreamingStarted() {
        super.onDreamingStarted()
        lifecycleRegistry.currentState = Lifecycle.State.RESUMED
    }

    override fun onDreamingStopped() {
        lifecycleRegistry.currentState = Lifecycle.State.STARTED
        super.onDreamingStopped()
    }

    override fun onDetachedFromWindow() {
        lifecycleRegistry.currentState = Lifecycle.State.DESTROYED
        viewModelStore.clear()
        super.onDetachedFromWindow()
    }
}
