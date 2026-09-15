# AudioLink R8 规则（release 开了 isMinifyEnabled + isShrinkResources）
#
# 为什么必须有这个文件：内核经 **JNA** 加载 libaudiolink_ffi.so 并做结构体/回调映射，
# 而 JNA 靠 **反射 + 名字** 找库与符号；UniFFI 生成的绑定同理。
# R8 默认会删掉「没有静态引用」的类/成员并改名 —— 那种包能装上，但一启动就
# UnsatisfiedLinkError / ClassNotFoundException，且只在 release 上复现。

# ---- JNA ----
-keep class com.sun.jna.** { *; }
-keep interface com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.Structure { *; }
-keep class * implements com.sun.jna.Library { *; }
-keep class * implements com.sun.jna.Callback { *; }
-dontwarn com.sun.jna.**

# ---- UniFFI 生成的绑定（含 UniffiLib : com.sun.jna.Library）----
-keep class com.gotkicry.audiolink.core.** { *; }

# ---- 崩溃栈可读性（代价很小）----
-keepattributes SourceFile,LineNumberTable
-renamesourcefileattribute SourceFile
