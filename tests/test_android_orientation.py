import unittest
import xml.etree.ElementTree as ET

from lib.android_project import manifest_xml


class AndroidOrientationTest(unittest.TestCase):
    def test_activity_orientation(self):
        for activity, expected in (
            ({"name": ".SubCraftActivity", "orientation": "unspecified"}, "user"),
            ({"name": ".WordCraftActivity", "orientation": "unspecified"}, "user"),
            ({}, "user"),
            ({"orientation": "landscape"}, "landscape"),
        ):
            with self.subTest(activity=activity):
                xml = manifest_xml({"application": {"activity": [activity]}})
                element = ET.fromstring(xml).find("application/activity")
                self.assertEqual(
                    element.get("{http://schemas.android.com/apk/res/android}screenOrientation"),
                    expected,
                )
