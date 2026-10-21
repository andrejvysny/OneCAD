import UIKit

/// Stable `contact_id`s for physical contacts.
///
/// UIKit reuses one UITouch object for a whole touch sequence, so its object
/// identity is the key while the touch is live. Coalesced and predicted
/// touches are separate objects and always use the owning touch's id.
/// Estimation updates can arrive after a touch ended, so ids that still
/// expect updates are also kept by `estimationUpdateIndex`.
@MainActor
struct ContactRegistry {
    private var live: [ObjectIdentifier: UInt64] = [:]
    private var pendingEstimation: [Int: UInt64] = [:]
    private var rollReporting: Set<UInt64> = []
    private var nextID: UInt64 = 1

    mutating func allocate() -> UInt64 {
        defer { nextID &+= 1 }
        return nextID
    }

    /// Creates the mapping on first sight so a touch that began before the
    /// view was ready still gets one stable id.
    mutating func contactID(for touch: UITouch) -> UInt64 {
        let key = ObjectIdentifier(touch)
        if let id = live[key] { return id }
        let id = allocate()
        live[key] = id
        return id
    }

    mutating func end(_ touch: UITouch) {
        if let id = live.removeValue(forKey: ObjectIdentifier(touch)) {
            rollReporting.remove(id)
        }
    }

    /// Hover contacts have no UITouch; their ids end through this call.
    mutating func forget(_ contact: UInt64) {
        rollReporting.remove(contact)
    }

    func liveID(for touch: UITouch) -> UInt64? {
        live[ObjectIdentifier(touch)]
    }

    mutating func noteEstimation(index: Int, contact: UInt64, expectsUpdate: Bool) {
        if expectsUpdate {
            pendingEstimation[index] = contact
        } else {
            pendingEstimation.removeValue(forKey: index)
        }
    }

    func contactForEstimation(index: Int) -> UInt64? {
        pendingEstimation[index]
    }

    /// Roll stays "present" for the rest of a contact once a nonzero value was
    /// seen; Pencils without roll sensing always report exactly zero.
    mutating func reportsRoll(_ contact: UInt64, roll: CGFloat) -> Bool {
        if roll != 0 { rollReporting.insert(contact) }
        return rollReporting.contains(contact)
    }

    mutating func reset() {
        live.removeAll()
        pendingEstimation.removeAll()
        rollReporting.removeAll()
    }
}
