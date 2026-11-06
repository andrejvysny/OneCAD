#pragma once

#include <bit>
#include <charconv>
#include <cmath>
#include <cstdint>
#include <iomanip>
#include <sstream>

#include "io/SourceFaceClaim.h"
#include "nlohmann/json.hpp"

namespace onecad::io {

inline bool source_lower_hex(const std::string& value, std::size_t size) {
    if (value.size() != size) return false;
    for (unsigned char c : value) if (!((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f'))) return false;
    return true;
}

inline bool source_producer_scope(const std::string& producer, const std::string& body) {
    if (producer.size() != 36) return false;
    for (std::size_t i = 0; i < producer.size(); ++i) {
        if (i == 8 || i == 13 || i == 18 || i == 23) {
            if (producer[i] != '-') return false;
        } else if (!((producer[i] >= '0' && producer[i] <= '9') ||
                     (producer[i] >= 'a' && producer[i] <= 'f'))) return false;
    }
    const std::string prefix = "body_" + producer;
    if (body == prefix) return true;
    if (!body.starts_with(prefix + ':')) return false;
    const auto ordinal = body.substr(prefix.size() + 1);
    if (ordinal.empty() || (ordinal.size() > 1 && ordinal[0] == '0')) return false;
    for (unsigned char c : ordinal) if (c < '0' || c > '9') return false;
    std::size_t value = 0;
    const auto parsed = std::from_chars(ordinal.data(), ordinal.data() + ordinal.size(), value);
    return parsed.ec == std::errc{} && parsed.ptr == ordinal.data() + ordinal.size();
}

inline bool source_xbf_label(const std::string& label) {
    if (label.size() > 256 || !label.starts_with("0:")) return false;
    bool digit = false;
    for (unsigned char c : label) {
        if (c == ':') {
            if (!digit) return false;
            digit = false;
        } else if (c >= '0' && c <= '9') {
            digit = true;
        } else return false;
    }
    return digit;
}

// Only a producer-verified claim can construct a certificate. Consumers compare
// the entire JSON object, including unknown fields, against this exact evidence.
inline nlohmann::json source_identity_json(const SourceFaceClaim& claim) {
    if (!source_lower_hex(claim.source_sha256, 64) ||
        !source_lower_hex(claim.kernel_fingerprint, 16) ||
        !source_producer_scope(claim.producer_id, claim.body_id) ||
        !std::isfinite(claim.unit_scale) || claim.unit_scale <= 0) return {};
    nlohmann::json address;
    if (claim.address_kind == SourceFaceClaim::AddressKind::StepEntity) {
        if (claim.codec != "step" || claim.codec_version != "occt-step-stream-v1" ||
            claim.reader_policy != "step-v1" || claim.step_root_entity <= 0 ||
            claim.step_face_entity <= 0 || !claim.step_occurrence_path.empty()) return {};
        address = {{"kind", "step"}, {"rootEntityLabel", claim.step_root_entity},
                   {"occurrencePath", nlohmann::json::array()}, {"faceEntityLabel", claim.step_face_entity}};
    } else {
        if (claim.codec != "xbf" || claim.codec_version != "12" || claim.reader_policy != "xbf-v12" ||
            !source_xbf_label(claim.xbf_root_label) || !source_xbf_label(claim.xbf_face_label) ||
            !claim.xbf_face_label.starts_with(claim.xbf_root_label + ':') ||
            !claim.xbf_occurrence_path.empty()) return {};
        address = {{"kind", "xbf"}, {"rootLabel", claim.xbf_root_label},
                   {"occurrencePath", nlohmann::json::array()}, {"faceLabel", claim.xbf_face_label}};
    }
    std::ostringstream scale;
    scale << std::hex << std::setfill('0') << std::setw(16) << std::bit_cast<std::uint64_t>(claim.unit_scale);
    return {{"version", 1}, {"sourceSha256", claim.source_sha256}, {"codec", claim.codec},
            {"codecVersion", claim.codec_version}, {"readerPolicy", claim.reader_policy},
            {"kernelFingerprint", claim.kernel_fingerprint}, {"producerRecordId", claim.producer_id},
            {"bodyId", claim.body_id}, {"unitScaleBits", scale.str()}, {"sourceAddress", std::move(address)}};
}

inline bool source_identity_json_equal(const nlohmann::json& expected, const nlohmann::json& value) {
    if (expected.is_number_integer()) return value.is_number_integer() && expected == value;
    if (expected.type() != value.type()) return false;
    if (expected.is_array()) {
        if (expected.size() != value.size()) return false;
        for (std::size_t i = 0; i < expected.size(); ++i)
            if (!source_identity_json_equal(expected[i], value[i])) return false;
        return true;
    }
    if (expected.is_object()) {
        if (expected.size() != value.size()) return false;
        for (auto it = expected.begin(); it != expected.end(); ++it) {
            const auto found = value.find(it.key());
            if (found == value.end() || !source_identity_json_equal(it.value(), *found)) return false;
        }
        return true;
    }
    return expected == value;
}

inline bool source_claim_same_identity(const SourceFaceClaim& a, const SourceFaceClaim& b) {
    return a.source_sha256 == b.source_sha256 && a.codec == b.codec &&
           a.codec_version == b.codec_version && a.reader_policy == b.reader_policy &&
           a.kernel_fingerprint == b.kernel_fingerprint && a.producer_id == b.producer_id &&
           a.body_id == b.body_id && std::bit_cast<std::uint64_t>(a.unit_scale) ==
                                      std::bit_cast<std::uint64_t>(b.unit_scale) &&
           a.address_kind == b.address_kind && a.step_root_entity == b.step_root_entity &&
           a.step_face_entity == b.step_face_entity && a.step_occurrence_path == b.step_occurrence_path &&
           a.xbf_root_label == b.xbf_root_label && a.xbf_face_label == b.xbf_face_label &&
           a.xbf_occurrence_path == b.xbf_occurrence_path;
}

inline bool source_identity_matches(const SourceFaceClaim& claim, const nlohmann::json& value) {
    const nlohmann::json expected = source_identity_json(claim);
    return !expected.is_null() && source_identity_json_equal(expected, value);
}

}  // namespace onecad::io
