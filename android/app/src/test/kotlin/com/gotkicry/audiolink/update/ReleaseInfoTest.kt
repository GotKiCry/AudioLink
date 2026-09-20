package com.gotkicry.audiolink.update

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 应用内更新的**纯逻辑**测试：版本比较、ABI 选包、API 解析。
 *
 * 为什么这些值得单测：它们的判据是「更新该不该提示」「该装哪个包」——错了的后果分别是
 * 「永远提示有新版」和「装了不匹配的 ABI」，两者都不会在编译期暴露。安全相关的部分
 * （签名校验、安装器拉起）依赖 Android 框架，只能靠真机 + 代码审查覆盖，不在这里假装测过。
 */
class ReleaseInfoTest {

    // ── 版本比较 ─────────────────────────────────────────────────────

    @Test
    fun newer_patch_is_greater() {
        assertTrue(VersionCompare.compare("0.1.1", "0.1.0") > 0)
    }

    @Test
    fun tag_prefix_and_prerelease_suffix_are_normalized() {
        assertEquals("0.1.1", VersionCompare.normalize("v0.1.1"))
        assertEquals("0.1.1", VersionCompare.normalize("0.1.1-beta.2"))
        assertEquals("0.1.1", VersionCompare.normalize("0.1.1+build.7"))
    }

    @Test
    fun missing_segments_count_as_zero() {
        assertEquals(0, VersionCompare.compare("0.2", "0.2.0"))
    }

    @Test
    fun double_digit_segments_are_not_lexicographic() {
        // 字符串比较会把 "0.1.10" 判成小于 "0.1.9" —— 这正是必须逐段转数字的原因。
        assertTrue(VersionCompare.compare("0.1.10", "0.1.9") > 0)
    }

    @Test
    fun major_rollover_beats_everything_below() {
        assertTrue(VersionCompare.compare("1.0.0", "0.99.99") > 0)
    }

    @Test
    fun garbage_never_throws() {
        // 版本号来自网络：不能因为对端写了个怪字符串就崩。
        assertEquals(0, VersionCompare.compare("abc", "0.0.0"))
        assertEquals(0, VersionCompare.compare("", ""))
    }

    // ── ABI 选包 ────────────────────────────────────────────────────

    private fun asset(name: String) = ReleaseAsset(name, "https://example.invalid/" + name, 1_024L)

    @Test
    fun picks_the_pack_matching_the_first_supported_abi() {
        val assets = listOf(
            asset("app-armeabi-v7a-release.apk"),
            asset("app-arm64-v8a-release.apk"),
        )
        val picked = ReleaseSelection.apkForAbis(assets, listOf("arm64-v8a", "armeabi-v7a"))
        assertEquals("app-arm64-v8a-release.apk", picked?.name)
    }

    @Test
    fun falls_back_to_the_next_supported_abi() {
        val assets = listOf(asset("app-armeabi-v7a-release.apk"))
        val picked = ReleaseSelection.apkForAbis(assets, listOf("arm64-v8a", "armeabi-v7a"))
        assertEquals("app-armeabi-v7a-release.apk", picked?.name)
    }

    @Test
    fun ignores_non_apk_assets() {
        // 同一次发布里还有 latest.json / .sig / 绿色版 zip —— 它们绝不能被抓来当安装包。
        val assets = listOf(
            asset("latest.json"),
            asset("AudioLink_0.1.1_x64-setup.exe"),
            asset("app-arm64-v8a-release.apk"),
            asset("app-arm64-v8a-release.apk.sig"),
        )
        val picked = ReleaseSelection.apkForAbis(assets, listOf("arm64-v8a"))
        assertEquals("app-arm64-v8a-release.apk", picked?.name)
    }

    @Test
    fun returns_null_when_no_abi_matches() {
        // 返回 null 让上层给出「这次发布没有适配你的设备的包」，而不是随便装一个。
        val assets = listOf(asset("app-x86_64-release.apk"))
        assertNull(ReleaseSelection.apkForAbis(assets, listOf("arm64-v8a", "armeabi-v7a")))
    }

    // ── GitHub API 解析 ─────────────────────────────────────────────

    private val sample = """
        {
          "tag_name": "v0.1.1",
          "name": "AudioLink 0.1.1",
          "body": "修了什么什么",
          "published_at": "2026-09-20T02:00:00Z",
          "assets": [
            { "name": "app-arm64-v8a-release.apk",
              "browser_download_url": "https://github.com/GotKiCry/AudioLink/releases/download/v0.1.1/app-arm64-v8a-release.apk",
              "size": 5903497 }
          ]
        }
    """.trimIndent()

    @Test
    fun parses_a_realistic_response() {
        val info = ReleaseParser.parse(sample)
        assertEquals("0.1.1", info.version)
        assertEquals("修了什么什么", info.notes)
        assertEquals("2026-09-20T02:00:00Z", info.publishedAt)
        assertEquals(1, info.assets.size)
        assertEquals(5_903_497L, info.assets[0].sizeBytes)
    }

    @Test
    fun missing_notes_and_assets_are_tolerated() {
        val info = ReleaseParser.parse("""{"tag_name":"v9.9.9"}""")
        assertEquals("9.9.9", info.version)
        assertEquals("", info.notes)
        assertNull(info.publishedAt)
        assertTrue(info.assets.isEmpty())
    }

    @Test
    fun missing_tag_is_an_error_not_a_silent_up_to_date() {
        // GitHub 若改了字段名，必须在解析这一步响 —— 静默当成「已是最新」是最坏的失败方式。
        assertThrows(IllegalStateException::class.java) {
            ReleaseParser.parse("""{"name":"x"}""")
        }
    }
}
