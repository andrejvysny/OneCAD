#pragma once

#include <cstdint>
#include <cerrno>
#include <fcntl.h>
#include <sys/stat.h>
#include <unistd.h>
#include <istream>
#include <limits>
#include <span>
#include <streambuf>
#include <string>
#include <vector>

#include "util/Hashing.h"

namespace onecad::io {

inline constexpr std::size_t kMaxImportSourceBytes = 256u * 1024u * 1024u;

struct CapturedSource {
    std::vector<std::uint8_t> bytes;
    std::string sha256;
    std::string error;
    bool ok() const { return error.empty(); }
};

inline CapturedSource capture_source(const std::string& path, const std::string& expected_sha) {
    CapturedSource out;
    const int fd = ::open(path.c_str(), O_RDONLY | O_NONBLOCK | O_CLOEXEC);
    if (fd < 0) { out.error = "source file is not readable"; return out; }
    struct CloseFd { int fd; ~CloseFd() { ::close(fd); } } close_fd{fd};
    struct stat st {};
    if (::fstat(fd, &st) != 0 || !S_ISREG(st.st_mode)) {
        out.error = "source is not a regular file";
        return out;
    }
    if (st.st_size < 0 || static_cast<std::uintmax_t>(st.st_size) > kMaxImportSourceBytes) {
        out.error = "source exceeds the 256 MiB admission limit";
        return out;
    }
    const auto size = static_cast<std::size_t>(st.st_size);
    out.bytes.resize(size);
    std::size_t offset = 0;
    while (offset < size) {
        const auto count = ::read(fd, out.bytes.data() + offset, size - offset);
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) {
            out.bytes.clear();
            out.error = "source changed or could not be read completely";
            return out;
        }
        offset += static_cast<std::size_t>(count);
    }
    std::uint8_t extra;
    ssize_t count;
    do { count = ::read(fd, &extra, 1); } while (count < 0 && errno == EINTR);
    if (count != 0) {
        out.bytes.clear();
        out.error = "source changed or could not be read completely";
        return out;
    }
    static constexpr std::uint8_t kEmpty = 0;
    out.sha256 = hashing::sha256_hex(out.bytes.empty() ? &kEmpty : out.bytes.data(), out.bytes.size());
    if (expected_sha.empty() || out.sha256 != expected_sha) {
        out.bytes.clear();
        out.error = "source SHA-256 does not match captured bytes";
    }
    return out;
}

class SourceSpanBuffer : public std::streambuf {
public:
    explicit SourceSpanBuffer(std::span<const std::uint8_t> bytes) {
        char* begin = bytes.empty() ? &empty_ : const_cast<char*>(reinterpret_cast<const char*>(bytes.data()));
        setg(begin, begin, begin + bytes.size());
    }

protected:
    pos_type seekoff(off_type offset, std::ios_base::seekdir dir,
                     std::ios_base::openmode mode) override {
        if ((mode & std::ios_base::in) == 0) return pos_type(off_type(-1));
        const auto size = egptr() - eback();
        const auto origin = dir == std::ios_base::beg ? 0 :
                            dir == std::ios_base::cur ? gptr() - eback() :
                            dir == std::ios_base::end ? size : -1;
        if (origin < 0 || offset < -origin || offset > size - origin) return pos_type(off_type(-1));
        const auto pos = origin + offset;
        setg(eback(), eback() + pos, egptr());
        return pos_type(pos);
    }
    pos_type seekpos(pos_type pos, std::ios_base::openmode mode) override {
        return seekoff(off_type(pos), std::ios_base::beg, mode);
    }

private:
    char empty_ = 0;
};

class SourceSpanStream : public std::istream {
public:
    explicit SourceSpanStream(std::span<const std::uint8_t> bytes)
        : std::istream(nullptr), buffer_(bytes) { rdbuf(&buffer_); }
private:
    SourceSpanBuffer buffer_;
};

}  // namespace onecad::io
