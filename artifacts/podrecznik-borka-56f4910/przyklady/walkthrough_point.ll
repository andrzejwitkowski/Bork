; ModuleID = 'bork'
source_filename = "bork"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-pc-linux-gnu"

%bork.Point = type { { ptr, i64 }, i32, i32 }

@str = private unnamed_addr constant [1 x i8] c"a"

define i32 @main() {
entry:
  %p = alloca { ptr, ptr }, align 8
  %arena = call ptr @bork_arena_push()
  %arena.root = call ptr @bork_arena_root()
  %arena.dynamic = call ptr @bork_arena_create_dynamic(ptr %arena.root, i8 0)
  call void @bork_arena_register_strong_root(ptr %arena, ptr %arena.dynamic)
  %object = call ptr @bork_arena_alloc(ptr %arena.dynamic, i64 ptrtoint (ptr getelementptr (%bork.Point, ptr null, i32 1) to i64), i64 ptrtoint (ptr getelementptr ({ i1, %bork.Point }, ptr null, i64 0, i32 1) to i64))
  %field = getelementptr inbounds nuw %bork.Point, ptr %object, i32 0, i32 0
  %move.dst = call ptr @bork_arena_alloc(ptr %arena.dynamic, i64 1, i64 1)
  call void @llvm.memcpy.p0.p0.i64(ptr align 1 %move.dst, ptr align 1 @str, i64 1, i1 false)
  %moved = insertvalue { ptr, i64 } { ptr @str, i64 1 }, ptr %move.dst, 0
  store { ptr, i64 } %moved, ptr %field, align 8
  %field1 = getelementptr inbounds nuw %bork.Point, ptr %object, i32 0, i32 1
  store i32 3, ptr %field1, align 4
  %field2 = getelementptr inbounds nuw %bork.Point, ptr %object, i32 0, i32 2
  store i32 4, ptr %field2, align 4
  %object.arena = insertvalue { ptr, ptr } zeroinitializer, ptr %arena.dynamic, 0
  %object.ptr = insertvalue { ptr, ptr } %object.arena, ptr %object, 1
  store { ptr, ptr } %object.ptr, ptr %p, align 8
  %object.arena3 = extractvalue { ptr, ptr } %object.ptr, 0
  %p4 = load { ptr, ptr }, ptr %p, align 8
  %call = call i32 @bork.sum(ptr %arena, { ptr, ptr } %p4)
  %p5 = load { ptr, ptr }, ptr %p, align 8
  %object.ptr6 = extractvalue { ptr, ptr } %p5, 1
  %field7 = getelementptr inbounds nuw %bork.Point, ptr %object.ptr6, i32 0, i32 0
  %field8 = load { ptr, i64 }, ptr %field7, align 8
  %len = extractvalue { ptr, i64 } %field8, 1
  %len.i32 = trunc i64 %len to i32
  %add = add i32 %call, %len.i32
  call void @bork_arena_pop(ptr %arena)
  ret i32 %add
}

define internal i32 @bork.sum(ptr %0, { ptr, ptr } %1) {
entry:
  %p = alloca { ptr, ptr }, align 8
  %arena = call ptr @bork_arena_push_child(ptr %0)
  store { ptr, ptr } %1, ptr %p, align 8
  %p1 = load { ptr, ptr }, ptr %p, align 8
  %object.ptr = extractvalue { ptr, ptr } %p1, 1
  %field = getelementptr inbounds nuw %bork.Point, ptr %object.ptr, i32 0, i32 1
  %field2 = load i32, ptr %field, align 4
  %p3 = load { ptr, ptr }, ptr %p, align 8
  %object.ptr4 = extractvalue { ptr, ptr } %p3, 1
  %field5 = getelementptr inbounds nuw %bork.Point, ptr %object.ptr4, i32 0, i32 2
  %field6 = load i32, ptr %field5, align 4
  %add = add i32 %field2, %field6
  call void @bork_arena_pop(ptr %arena)
  ret i32 %add
}

declare ptr @bork_arena_push()

declare ptr @bork_arena_root()

declare ptr @bork_arena_create_dynamic(ptr, i8)

declare void @bork_arena_register_strong_root(ptr, ptr)

declare ptr @bork_arena_alloc(ptr, i64, i64)

; Function Attrs: nocallback nofree nosync nounwind willreturn memory(argmem: readwrite)
declare void @llvm.memcpy.p0.p0.i64(ptr noalias writeonly captures(none), ptr noalias readonly captures(none), i64, i1 immarg) #0

declare void @bork_arena_pop(ptr)

declare ptr @bork_arena_push_child(ptr)

attributes #0 = { nocallback nofree nosync nounwind willreturn memory(argmem: readwrite) }
