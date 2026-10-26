package tv.lounge.launcher.di

import android.content.Context
import dagger.Module
import dagger.Provides
import dagger.hilt.InstallIn
import dagger.hilt.android.qualifiers.ApplicationContext
import dagger.hilt.components.SingletonComponent
import tv.lounge.collector.apps.AppsCollector
import tv.lounge.collector.inputs.InputsCollector
import tv.lounge.collector.media.MediaSessionsCollector
import tv.lounge.collector.tvprovider.TvProviderCollector
import tv.lounge.launcher.LoungeApplication
import tv.lounge.platform.AndroidPlatform
import tv.lounge.platform.Prefs
import javax.inject.Singleton

@Module
@InstallIn(SingletonComponent::class)
object CoreModule {

    @Provides
    @Singleton
    fun platform(@ApplicationContext context: Context): AndroidPlatform = (context as LoungeApplication).platform

    @Provides
    @Singleton
    fun appsCollector(@ApplicationContext context: Context): AppsCollector = AppsCollector(context)

    @Provides
    @Singleton
    fun prefs(@ApplicationContext context: Context): Prefs = (context as LoungeApplication).prefs

    @Provides
    @Singleton
    fun power(@ApplicationContext context: Context): tv.lounge.power.PowerSetup = (context as LoungeApplication).power

    @Provides
    @Singleton
    fun updates(@ApplicationContext context: Context): tv.lounge.update.UpdateManager = (context as LoungeApplication).updates

    @Provides
    @Singleton
    fun tvProvider(@ApplicationContext context: Context): TvProviderCollector = TvProviderCollector(context)

    @Provides
    @Singleton
    fun inputs(@ApplicationContext context: Context): InputsCollector = InputsCollector(context)

    @Provides
    @Singleton
    fun mediaSessions(@ApplicationContext context: Context, platform: AndroidPlatform): MediaSessionsCollector =
        MediaSessionsCollector(context).also { platform.mediaSender = it::send }
}
