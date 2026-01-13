#ifndef ONECAD_TEST_HARNESS_H
#define ONECAD_TEST_HARNESS_H

#include <cmath>
#include <functional>
#include <iostream>
#include <sstream>
#include <string>
#include <string_view>
#include <vector>

namespace onecad::test {

struct TestCase {
    std::string name;
    std::function<void()> func;
};

namespace detail {

template <typename T>
std::string toString(const T& value) {
    if constexpr (requires(std::ostream& os, const T& v) { os << v; }) {
        std::ostringstream oss;
        oss << value;
        return oss.str();
    } else {
        return "<unprintable>";
    }
}

} // namespace detail

inline std::vector<TestCase>& registry() {
    static std::vector<TestCase> r;
    return r;
}

inline int& failureCount() {
    static int failures = 0;
    return failures;
}

inline void recordFailure(std::string_view expr, std::string_view file, int line, std::string_view message) {
    std::cerr << "[FAIL] " << file << ":" << line << " - " << expr;
    if (!message.empty()) {
        std::cerr << " | " << message;
    }
    std::cerr << std::endl;
    ++failureCount();
}

inline void recordSuccess(std::string_view /*expr*/, std::string_view /*file*/, int /*line*/) {
    // Intentionally no-op for now; keep output concise
}

struct Registrar {
    Registrar(std::string name, std::function<void()> func) {
        registry().push_back({std::move(name), std::move(func)});
    }
};

#define TEST_CASE(name) \
    static void name(); \
    static ::onecad::test::Registrar registrar_##name(#name, name); \
    static void name()

#define EXPECT_TRUE(expr) do { \
    if (!(expr)) { \
        ::onecad::test::recordFailure(#expr, __FILE__, __LINE__, "expected true"); \
    } else { \
        ::onecad::test::recordSuccess(#expr, __FILE__, __LINE__); \
    } \
} while (0)

#define EXPECT_FALSE(expr) do { \
    if (expr) { \
        ::onecad::test::recordFailure(#expr, __FILE__, __LINE__, "expected false"); \
    } else { \
        ::onecad::test::recordSuccess(#expr, __FILE__, __LINE__); \
    } \
} while (0)

#define EXPECT_EQ(a, b) do { \
    auto _va = (a); \
    auto _vb = (b); \
    if (!(_va == _vb)) { \
        std::string lhs = ::onecad::test::detail::toString(_va); \
        std::string rhs = ::onecad::test::detail::toString(_vb); \
        std::ostringstream oss; \
        oss << "lhs=" << lhs << " rhs=" << rhs; \
        ::onecad::test::recordFailure(#a " == " #b, __FILE__, __LINE__, oss.str()); \
    } else { \
        ::onecad::test::recordSuccess(#a " == " #b, __FILE__, __LINE__); \
    } \
} while (0)

#define EXPECT_NE(a, b) do { \
    auto _va = (a); \
    auto _vb = (b); \
    if (_va == _vb) { \
        std::string lhs = ::onecad::test::detail::toString(_va); \
        std::string rhs = ::onecad::test::detail::toString(_vb); \
        std::ostringstream oss; \
        oss << "lhs=" << lhs << " rhs=" << rhs; \
        ::onecad::test::recordFailure(#a " != " #b, __FILE__, __LINE__, oss.str()); \
    } else { \
        ::onecad::test::recordSuccess(#a " != " #b, __FILE__, __LINE__); \
    } \
} while (0)

#define EXPECT_NEAR(a, b, tol) do { \
    auto _va = (a); \
    auto _vb = (b); \
    auto _tol = (tol); \
    double diff = std::fabs(static_cast<double>(_va) - static_cast<double>(_vb)); \
    double scale = std::max(std::fabs(static_cast<double>(_va)), std::fabs(static_cast<double>(_vb))); \
    if (!(diff <= _tol || diff <= _tol * scale)) { \
        std::ostringstream oss; \
        oss << "lhs=" << _va << " rhs=" << _vb << " diff=" << diff << " tol=" << _tol; \
        ::onecad::test::recordFailure(#a " ~= " #b, __FILE__, __LINE__, oss.str()); \
    } else { \
        ::onecad::test::recordSuccess(#a " ~= " #b, __FILE__, __LINE__); \
    } \
} while (0)

#define EXPECT_VEC2_NEAR(a, b, tol) do { \
    auto _va = (a); \
    auto _vb = (b); \
    double dx = static_cast<double>(_va.x) - static_cast<double>(_vb.x); \
    double dy = static_cast<double>(_va.y) - static_cast<double>(_vb.y); \
    double dist = std::sqrt(dx * dx + dy * dy); \
    double _tol = (tol); \
    if (dist > _tol) { \
        std::ostringstream oss; \
        oss << "dist=" << dist << " tol=" << _tol << " ax=" << _va.x << " ay=" << _va.y \
            << " bx=" << _vb.x << " by=" << _vb.y; \
        ::onecad::test::recordFailure(#a " ~= " #b, __FILE__, __LINE__, oss.str()); \
    } else { \
        ::onecad::test::recordSuccess(#a " ~= " #b, __FILE__, __LINE__); \
    } \
} while (0)

inline int runAllTests() {
    int passed = 0;
    auto& r = registry();
    for (const auto& tc : r) {
        try {
            tc.func();
            ++passed;
        } catch (const std::exception& ex) {
            std::cerr << "[EXCEPT] " << tc.name << " - " << ex.what() << std::endl;
            ++failureCount();
        } catch (...) {
            std::cerr << "[EXCEPT] " << tc.name << " - unknown exception" << std::endl;
            ++failureCount();
        }
    }

    std::cout << "[RESULT] " << passed << "/" << r.size() << " test cases executed" << std::endl;
    if (failureCount() == 0) {
        std::cout << "[PASS] All tests passed" << std::endl;
    } else {
        std::cout << "[FAIL] " << failureCount() << " failures" << std::endl;
    }
    return failureCount();
}

} // namespace onecad::test

#endif // ONECAD_TEST_HARNESS_H
