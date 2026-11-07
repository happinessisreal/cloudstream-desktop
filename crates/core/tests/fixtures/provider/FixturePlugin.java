package fixture;

import com.lagradost.cloudstream3.*;
import com.lagradost.cloudstream3.plugins.*;
import com.lagradost.cloudstream3.utils.*;
import kotlin.Unit;
import kotlin.coroutines.Continuation;
import kotlin.jvm.functions.Function1;
import java.util.*;

@CloudstreamPlugin
public class FixturePlugin extends BasePlugin {
    @Override public void load() { registerMainAPI(new FixtureProvider()); }
    public static class FixtureProvider extends MainAPI {
        @Override public String getName() { return "Isolated resource fixture"; }
        @Override public String getMainUrl() { return "http://127.0.0.1"; }
        @Override public Object loadLinks(String base, boolean cast,
                Function1<? super SubtitleFile, Unit> subtitle,
                Function1<? super ExtractorLink, Unit> callback,
                Continuation<? super Boolean> continuation) {
            SubtitleFile sub = new SubtitleFile("English", base + "/sub.vtt");
            sub.setHeaders(Map.of("X-Resource", "subtitle"));
            subtitle.invoke(sub);
            SubtitleFile archive = new SubtitleFile("Archive English", base + "/subs.zip?token=opaque");
            archive.setHeaders(Map.of("X-Resource", "archive"));
            subtitle.invoke(archive);
            AudioFile audio = new AudioFile(base + "/audio.wav", Map.of("X-Resource", "audio"));
            ExtractorLink link = new ExtractorLink("fixture", "Protected", base + "/video.wav", "", 720,
                    Map.of("X-Resource", "video"), null, ExtractorLinkType.VIDEO, List.of(audio));
            callback.invoke(link);
            return Boolean.TRUE;
        }
    }
}
