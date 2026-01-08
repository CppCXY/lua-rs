cmake_minimum_required(VERSION 3.11)

project(lua54 VERSION 5.5.0 LANGUAGES C)

# Options
option(LUA_BUILD_AS_DLL "Build Lua as shared library" OFF)
option(LUA_BUILD_INTERPRETER "Build Lua interpreter executable" ON)
option(LUA_BUILD_COMPILER "Build Lua compiler executable" ON)

# Platform definitions
if(WIN32)
    add_definitions(-D_CRT_SECURE_NO_WARNINGS)
elseif(CMAKE_SYSTEM_NAME MATCHES "Linux")
    add_definitions(-DLUA_USE_LINUX)
    set(LUA_LINK_LIBRARIES readline dl m)
else()
    add_definitions(-DLUA_USE_MACOSX)
    set(LUA_LINK_LIBRARIES readline)
endif()

# Core Lua sources (without lua.c and luac.c)
set(LUA_CORE_SOURCES
    ${lua54_SOURCE_DIR}/src/lapi.c
    ${lua54_SOURCE_DIR}/src/lauxlib.c
    ${lua54_SOURCE_DIR}/src/lbaselib.c
    ${lua54_SOURCE_DIR}/src/lcode.c
    ${lua54_SOURCE_DIR}/src/lcorolib.c
    ${lua54_SOURCE_DIR}/src/lctype.c
    ${lua54_SOURCE_DIR}/src/ldblib.c
    ${lua54_SOURCE_DIR}/src/ldebug.c
    ${lua54_SOURCE_DIR}/src/ldo.c
    ${lua54_SOURCE_DIR}/src/ldump.c
    ${lua54_SOURCE_DIR}/src/lfunc.c
    ${lua54_SOURCE_DIR}/src/lgc.c
    ${lua54_SOURCE_DIR}/src/linit.c
    ${lua54_SOURCE_DIR}/src/liolib.c
    ${lua54_SOURCE_DIR}/src/llex.c
    ${lua54_SOURCE_DIR}/src/lmathlib.c
    ${lua54_SOURCE_DIR}/src/lmem.c
    ${lua54_SOURCE_DIR}/src/loadlib.c
    ${lua54_SOURCE_DIR}/src/lobject.c
    ${lua54_SOURCE_DIR}/src/lopcodes.c
    ${lua54_SOURCE_DIR}/src/loslib.c
    ${lua54_SOURCE_DIR}/src/lparser.c
    ${lua54_SOURCE_DIR}/src/lstate.c
    ${lua54_SOURCE_DIR}/src/lstring.c
    ${lua54_SOURCE_DIR}/src/lstrlib.c
    ${lua54_SOURCE_DIR}/src/ltable.c
    ${lua54_SOURCE_DIR}/src/ltablib.c
    ${lua54_SOURCE_DIR}/src/ltm.c
    ${lua54_SOURCE_DIR}/src/lundump.c
    ${lua54_SOURCE_DIR}/src/lutf8lib.c
    ${lua54_SOURCE_DIR}/src/lvm.c
    ${lua54_SOURCE_DIR}/src/lzio.c
)

# Build library
if(LUA_BUILD_AS_DLL)
    add_library(lua54 SHARED ${LUA_CORE_SOURCES})
    if(WIN32)
        target_compile_definitions(lua54 PUBLIC -DLUA_BUILD_AS_DLL)
    endif()
else()
    add_library(lua54 STATIC ${LUA_CORE_SOURCES})
endif()

if(NOT WIN32)
    target_compile_options(lua54 PUBLIC -fPIC)
endif()

target_link_libraries(lua54 PUBLIC ${LUA_LINK_LIBRARIES})

# Build Lua interpreter executable
if(LUA_BUILD_INTERPRETER)
    add_executable(lua ${lua54_SOURCE_DIR}/src/lua.c)
    target_link_libraries(lua PRIVATE lua54)
    if(WIN32)
        # On Windows, link with lua54 library
    else()
        target_link_libraries(lua PRIVATE ${LUA_LINK_LIBRARIES})
    endif()
endif()

# Build Lua compiler executable
if(LUA_BUILD_COMPILER)
    add_executable(luac ${lua54_SOURCE_DIR}/src/luac.c)
    target_link_libraries(luac PRIVATE lua54)
    if(NOT WIN32)
        target_link_libraries(luac PRIVATE ${LUA_LINK_LIBRARIES})
    endif()
endif()

# Install targets
install(TARGETS lua54 DESTINATION lib)
if(LUA_BUILD_INTERPRETER)
    install(TARGETS lua DESTINATION bin)
endif()
if(LUA_BUILD_COMPILER)
    install(TARGETS luac DESTINATION bin)
endif()
install(FILES
    ${lua54_SOURCE_DIR}/src/lua.h
    ${lua54_SOURCE_DIR}/src/luaconf.h
    ${lua54_SOURCE_DIR}/src/lualib.h
    ${lua54_SOURCE_DIR}/src/lauxlib.h
    ${lua54_SOURCE_DIR}/src/lua.hpp
    DESTINATION include
)
